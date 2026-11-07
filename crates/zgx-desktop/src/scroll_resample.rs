//! Display-paced scrolling.
//!
//! Trackpads report motion on their own clock, out of phase with the display:
//! applying each event on arrival moves content 0, 1 or 2 events' worth per
//! refresh, which reads as judder even at full frame rate. Continuous (pixel)
//! input is instead resampled between buffered events. Irregular or sparse
//! input stays one interval behind; a stable stream at a multiple of the
//! display rate gradually moves to half an interval for less finger latency.
//! At a slow refresh (60 Hz) events keep up with frames, and being a whole
//! interval behind costs more than it evens out: a short, fixed time instead.
//! Discrete wheel notches have no motion curve of
//! their own; they ease towards their destination over a few refreshes.
use std::{collections::VecDeque, time::Duration, time::Instant};

/// A pause this long ends a stroke: whatever is still buffered is applied.
const STALL: Duration = Duration::from_millis(50);
/// Refreshes keep running this long after the last input, so a gesture never
/// stops and restarts the display link (a lost refresh each time) when the
/// applied motion momentarily catches up with the input.
const LINGER: Duration = Duration::from_millis(120);
/// Bounds for the sampling delay, which tracks the input interval. The most is
/// past the gap of events 30 a second, what a trackpad's slow end sends: at 20 ms
/// a frame ran out between them, stopped, and jumped, a 30 Hz rhythm at any frame
/// rate. Real events at 30 a second and 120 Hz: 37-44% of steps alternating and
/// 9-12% near nothing at 20 ms, against 8-13% and 0.4%.
const MIN_DELAY: Duration = Duration::from_millis(4);
const MAX_DELAY: Duration = Duration::from_millis(40);
/// The most an irregular stream stands behind: `MAX_DELAY` is for steady ones
/// (`STEADY_EVENTS` in a row within `STEADY_SPREAD` of the interval), slow
/// devices sending as evenly as a refresh. A hand's events wander (from the
/// average interval by 25% at the median, 50-70% at p75), and 40 ms behind them
/// took the slowest frames' lag from 24 to 32 ms on real recordings, where 20
/// keeps it at 28 and the steady 30-a-second stream even.
const STALL_BRIDGE: Duration = Duration::from_millis(20);
const STEADY_SPREAD: f32 = 0.5;
const STEADY_EVENTS: u8 = 4;
/// Refreshes this long or longer stand `SLOW_REFRESH_DELAY` behind what has
/// come, not an event interval, while events come within `SLOW_EVENTS` of them.
pub(crate) const SLOW_REFRESH: Duration = Duration::from_millis(12);
/// Real trackpad events at 60 Hz, position behind the read that carried it (p50)
/// and gaps of 25 ms or more between frames that moved, in a window: an interval
/// behind, 19 ms and 1.7%; this, 13 ms and 4.6%; none, 5 ms and 12%, a frame in
/// eight showing nothing new.
const SLOW_REFRESH_DELAY: Duration = Duration::from_millis(8);
/// Events further apart than this many refreshes are still bridged: a slow
/// device's motion would otherwise show every few frames. At 2, real events 30 a
/// second at 60 Hz stairstepped: 26% of gaps 25 ms or more, against 0.1% here.
const SLOW_EVENTS: f32 = 1.5;
/// Require a run of regular events before shortening the sampling delay.
const STABLE_INTERVALS: u8 = 8;
/// Wheel notches close this fraction of the remaining distance per second, in
/// the exponent: about 95% within 100 ms.
const WHEEL_RATE: f32 = 30.;
/// Notches this close are a wheel spinning. Closing a share of what's left
/// makes speed follow the distance left, which each notch tops up, so a spin
/// would pump at the notches' own rate: in one, each notch's distance is
/// handed over evenly until the next is due. Further apart they're steps of
/// their own, each starting at once, as a browser's wheel animations (200 ms
/// and more) stop overlapping too.
const WHEEL_SPIN: Duration = Duration::from_millis(120);

/// `ZGX_SCROLL_RESAMPLE=0` applies wheel input on arrival instead.
pub(crate) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| zgx::env::var_os("ZGX_SCROLL_RESAMPLE").is_none_or(|v| v != "0"))
}

#[derive(Default)]
pub(crate) struct ScrollResampler {
    /// Arrival time and cumulative position of each buffered event.
    points: VecDeque<(Instant, (f32, f32))>,
    /// Cumulative continuous motion received and applied.
    received: (f32, f32),
    applied: (f32, f32),
    /// Smoothed spacing between continuous events.
    interval: Option<Duration>,
    /// The time between refreshes, as last sampled.
    refresh: Duration,
    stable_intervals: u8,
    /// Events in a row within `STEADY_SPREAD` of the interval (`delay`).
    steady: u8,
    sample_delay: Option<Duration>,
    /// Wheel distance not yet applied, and when it last advanced.
    wheel: (f32, f32),
    wheel_at: Option<Instant>,
    /// Of that, what a spin hasn't handed over yet, how much of it a second,
    /// when the last notch came, and the smoothed spacing between a spin's.
    wheel_held: (f32, f32),
    wheel_rate: (f32, f32),
    wheel_notch: Option<Instant>,
    wheel_gap: Option<Duration>,
    /// Wheel notches move whole on the next refresh, not eased over several:
    /// an `NSScrollView`'s do (`set_wheel_eases`).
    whole_notches: bool,
    /// When the latest input arrived.
    last_input: Option<Instant>,
    /// Pointer position of the latest input; motion is delivered there.
    pub position: (f32, f32),
}

impl ScrollResampler {
    /// Whether wheel notches ease in over the next refreshes, as a browser's
    /// do (the default), or each moves whole on the next refresh, as an
    /// `NSScrollView`'s do: measured with real wheel events into a scroll
    /// view, a table and a text view, every notch moved its whole step on
    /// the refresh after it, and notches within one refresh together.
    pub fn set_wheel_eases(&mut self, eases: bool) {
        self.whole_notches = !eases;
    }
    pub fn pending(&self) -> bool {
        self.received != self.applied || self.wheel != (0., 0.)
    }
    /// A gesture is under way: refreshes should keep coming at full rate.
    pub fn active(&self, now: Instant) -> bool {
        self.pending()
            || self
                .last_input
                .is_some_and(|at| now.duration_since(at) < LINGER)
    }
    /// Buffer pixel-precise motion that arrived at `now`.
    pub fn push_precise(&mut self, now: Instant, delta: (f32, f32)) {
        let last = self.last_input.replace(now);
        match last {
            Some(last) if now.duration_since(last) < STALL => {
                let gap = now.duration_since(last).max(Duration::from_micros(500));
                self.steady = match self.interval {
                    Some(interval) if gap.abs_diff(interval) <= interval.mul_f32(STEADY_SPREAD) => {
                        self.steady.saturating_add(1)
                    }
                    _ => 0,
                };
                self.stable_intervals = match self.interval {
                    Some(interval)
                        if gap.abs_diff(interval)
                            <= (interval / 10).max(Duration::from_micros(500)) =>
                    {
                        self.stable_intervals
                            .saturating_add(1)
                            .min(STABLE_INTERVALS)
                    }
                    _ => 0,
                };
                self.interval = Some(match self.interval {
                    Some(interval) => interval.mul_f32(0.7) + gap.mul_f32(0.3),
                    None => gap,
                });
            }
            _ => {
                // A new stroke: the first event's motion spans one interval
                // before it, not the whole idle gap.
                let delay = self.delay();
                self.stable_intervals = 0;
                // Events as far apart as a stall, but as far apart as ever:
                // still a steady stream.
                let gap = last.map(|last| now.saturating_duration_since(last));
                self.steady = match (gap, self.interval) {
                    (Some(gap), Some(interval))
                        if gap.abs_diff(interval) <= interval.mul_f32(STEADY_SPREAD) =>
                    {
                        self.steady.saturating_add(1)
                    }
                    _ => 0,
                };
                self.sample_delay = None;
                self.points.clear();
                // A stalled frame may not have delivered the previous event
                // yet. Keep that debt: the next sample (or flush) pays it.
                self.points
                    .push_back((now.checked_sub(delay).unwrap_or(now), self.received));
            }
        }
        self.received.0 += delta.0;
        self.received.1 += delta.1;
        self.points.push_back((now, self.received));
    }
    /// Queue a discrete wheel step to ease in over the next refreshes.
    pub fn push_wheel(&mut self, now: Instant, delta: (f32, f32)) {
        self.last_input = Some(now);
        if self.wheel == (0., 0.) {
            self.wheel_at = Some(now);
        }
        self.wheel.0 += delta.0;
        self.wheel.1 += delta.1;
        let gap = self
            .wheel_notch
            .replace(now)
            .map(|last| now.duration_since(last))
            .filter(|gap| *gap < WHEEL_SPIN && !gap.is_zero());
        // (Smoothed as continuous input's spacing is: a hand's notches aren't even.)
        self.wheel_gap = gap.map(|gap| match self.wheel_gap {
            Some(smooth) => smooth.mul_f32(0.7) + gap.mul_f32(0.3),
            None => gap,
        });
        let gap = self.wheel_gap;
        for (held, rate, delta) in [
            (&mut self.wheel_held.0, &mut self.wheel_rate.0, delta.0),
            (&mut self.wheel_held.1, &mut self.wheel_rate.1, delta.1),
        ] {
            match gap {
                // (Turning back ends a spin: what was held goes on as a step.)
                Some(gap) if delta != 0. && delta.signum() == wheel_direction(*held, delta) => {
                    *held += delta;
                    *rate = held.abs() / gap.as_secs_f32();
                }
                _ => {
                    *held = 0.;
                    *rate = 0.;
                }
            }
        }
    }
    /// Deliver everything at once (no display refreshes are coming).
    pub fn flush(&mut self) -> Option<(f32, f32)> {
        let delta = (
            self.received.0 - self.applied.0 + self.wheel.0,
            self.received.1 - self.applied.1 + self.wheel.1,
        );
        self.applied = self.received;
        self.points.clear();
        self.wheel = (0., 0.);
        self.wheel_at = None;
        self.wheel_held = (0., 0.);
        self.wheel_rate = (0., 0.);
        self.wheel_notch = None;
        self.wheel_gap = None;
        self.stable_intervals = 0;
        self.sample_delay = None;
        (delta != (0., 0.)).then_some(delta)
    }
    /// The motion to apply at a display refresh processed at `now`.
    pub fn sample(&mut self, now: Instant, refresh_interval: Duration) -> Option<(f32, f32)> {
        self.refresh = refresh_interval;
        let full_delay = self.delay();
        let aligned = self.interval.is_some_and(|interval| {
            let ratio = refresh_interval.as_secs_f64() / interval.as_secs_f64();
            let multiple = ratio.round().max(1.);
            (ratio - multiple).abs() <= multiple * 0.1
        });
        let desired = if self.stable_intervals == STABLE_INTERVALS && aligned {
            full_delay / 2
        } else {
            full_delay
        };
        // Approach a shorter delay over several refreshes, rather than
        // jumping the content ahead. Jitter restores the safe delay at once;
        // `advance` below prevents that older horizon moving content back.
        let delay = self
            .sample_delay
            .unwrap_or(full_delay)
            .saturating_sub(refresh_interval / 10)
            .max(desired)
            .min(full_delay);
        self.sample_delay = Some(delay);
        let target = match self.points.back() {
            Some((last, total)) if now.duration_since(*last) >= STALL => *total,
            Some(_) => self.position_at(now.checked_sub(delay).unwrap_or(now)),
            None => self.received,
        };
        // Never step back: a late sample must not undo motion already shown.
        let continuous = (
            advance(self.applied.0, target.0, self.received.0),
            advance(self.applied.1, target.1, self.received.1),
        );
        let mut delta = (continuous.0 - self.applied.0, continuous.1 - self.applied.1);
        self.applied = continuous;
        // Keep one point at or before the sample time for interpolation.
        let horizon = now.checked_sub(delay).unwrap_or(now);
        while self.points.len() > 1 && self.points[1].0 <= horizon {
            self.points.pop_front();
        }
        if self.whole_notches && self.wheel != (0., 0.) {
            delta.0 += std::mem::take(&mut self.wheel.0);
            delta.1 += std::mem::take(&mut self.wheel.1);
            self.wheel_at = None;
            self.wheel_held = (0., 0.);
        }
        if let Some(at) = self.wheel_at.replace(now) {
            let elapsed = now.duration_since(at).as_secs_f32();
            let share = 1. - (-WHEEL_RATE * elapsed).exp();
            for (remaining, held, rate, out) in [
                (
                    &mut self.wheel.0,
                    &mut self.wheel_held.0,
                    &self.wheel_rate.0,
                    &mut delta.0,
                ),
                (
                    &mut self.wheel.1,
                    &mut self.wheel_held.1,
                    &self.wheel_rate.1,
                    &mut delta.1,
                ),
            ] {
                let release = (rate * elapsed).min(held.abs()).copysign(*held);
                *held -= release;
                // Whole pixels: content stays on the pixel grid, and the
                // offset sums exactly even tens of thousands of pixels down,
                // where an f32 holds fractions of a pixel only roughly.
                let step = if remaining.abs() <= 1. {
                    *remaining
                } else {
                    let step = ((*remaining - *held) * share).round();
                    let step = if step == 0. { remaining.signum() } else { step };
                    step.clamp(-remaining.abs(), remaining.abs())
                };
                *remaining -= step;
                *out += step;
                if *remaining == 0. {
                    *held = 0.;
                }
            }
            if self.wheel == (0., 0.) {
                self.wheel_at = None;
            }
        }
        (delta != (0., 0.)).then_some(delta)
    }
    fn delay(&self) -> Duration {
        // (Events closer than that keep their interval: it's as short, and even.)
        if self.refresh >= SLOW_REFRESH
            && self.interval.is_some_and(|interval| {
                interval >= SLOW_REFRESH && interval <= self.refresh.mul_f32(SLOW_EVENTS)
            })
        {
            return SLOW_REFRESH_DELAY;
        }
        let most = match self.steady >= STEADY_EVENTS {
            true => MAX_DELAY,
            false => STALL_BRIDGE,
        };
        self.interval
            .unwrap_or(Duration::from_millis(8))
            .clamp(MIN_DELAY, most)
    }
    /// The margin chosen by the last refresh, for opt-in presentation logs.
    pub fn sampling_delay(&self) -> Duration {
        self.sample_delay.unwrap_or_else(|| self.delay())
    }
    /// Cumulative position at `time`, linear between the events around it.
    fn position_at(&self, time: Instant) -> (f32, f32) {
        let mut previous = match self.points.front() {
            Some((at, position)) if time <= *at => return *position,
            Some(point) => *point,
            None => return self.received,
        };
        for &(at, position) in self.points.iter().skip(1) {
            if time <= at {
                let span = at.duration_since(previous.0).as_secs_f32();
                let t = if span > 0. {
                    time.duration_since(previous.0).as_secs_f32() / span
                } else {
                    1.
                };
                return (
                    previous.1.0 + (position.0 - previous.1.0) * t,
                    previous.1.1 + (position.1 - previous.1.1) * t,
                );
            }
            previous = (at, position);
        }
        previous.1
    }
}

/// The direction a wheel's going: what it holds, or else `delta` (nothing held
/// yet is no turn).
fn wheel_direction(held: f32, delta: f32) -> f32 {
    if held != 0. {
        held.signum()
    } else {
        delta.signum()
    }
}

/// Move from `applied` towards `target` without passing `received` or
/// reversing the direction of travel.
fn advance(applied: f32, target: f32, received: f32) -> f32 {
    if received >= applied {
        target.clamp(applied, received)
    } else {
        target.clamp(received, applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: Duration = Duration::from_millis(1);

    /// What notches (ms from a start, points) make of each refresh at 120 Hz;
    /// all of it landed, in whole points.
    fn wheel_steps(notches: &[(u64, f32)], refreshes: u32) -> Vec<f32> {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        let mut next = notches.iter().peekable();
        let mut steps = Vec::new();
        for i in 1..refreshes {
            let now = start + Duration::from_micros(8333) * i;
            while let Some((at, points)) = next.peek() {
                let at = start + Duration::from_millis(*at);
                if at > now {
                    break;
                }
                resampler.push_wheel(at, (0., *points));
                next.next();
            }
            steps.push(
                resampler
                    .sample(now, Duration::from_micros(8333))
                    .unwrap_or_default()
                    .1,
            );
        }
        assert!(!resampler.pending(), "{steps:?}");
        assert!(steps.iter().all(|step| step.fract() == 0.), "{steps:?}");
        steps
    }

    /// However fast the spin: past its start, steps stay near their mean.
    #[test]
    fn a_wheel_spun_steadily_at_any_rate_moves_steadily() {
        for gap in [25, 40, 60, 80, 100] {
            let count = 1600 / gap;
            let notches: Vec<_> = (0..count).map(|i| (i * gap, 42.)).collect();
            let steps = wheel_steps(&notches, 400);
            let steady = &steps[30..(1600 / 9) as usize];
            let mean = steady.iter().sum::<f32>() / steady.len() as f32;
            for step in steady {
                assert!(
                    (step / mean - 1.).abs() <= 0.2,
                    "every {gap} ms, {step} around {mean}: {steady:?}"
                );
            }
        }
    }

    /// A hand's notches aren't even: speed still holds far better than an
    /// ease of what's left, which at these gaps swings by 38% to 71%.
    #[test]
    fn a_wheel_spun_unevenly_stays_near_its_speed() {
        let mut seed = 88172645463325252_u64;
        for (gap, spread, most) in [(40, 15, 0.22), (60, 25, 0.28), (80, 40, 0.38)] {
            let mut swing = 0.;
            for _ in 0..8 {
                let (mut at, mut notches) = (0, Vec::new());
                while at < 2000 {
                    notches.push((at, 42.));
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    at += gap - spread + seed % (spread * 2);
                }
                let steps = wheel_steps(&notches, 400);
                let steady = &steps[30..240];
                let mean = steady.iter().sum::<f32>() / steady.len() as f32;
                let variance = steady.iter().map(|s| (s - mean) * (s - mean)).sum::<f32>();
                swing += (variance / steady.len() as f32).sqrt() / mean / 8.;
            }
            assert!(swing <= most, "around {gap} ms: {swing}");
        }
    }

    /// A notch on its own moves at once, as fast as it ever did, and a spin's
    /// first notch too: smoothing a spin costs no start.
    #[test]
    fn a_lone_notch_and_a_spins_first_start_at_once() {
        let lone = wheel_steps(&[(0, 42.)], 60);
        let spin: Vec<_> = (0..20).map(|i| (i * 40, 42.)).collect();
        let spin = wheel_steps(&spin, 200);
        for steps in [&lone, &spin] {
            // A fifth of its distance in the first refresh.
            assert!(steps[0] >= 8., "{steps:?}");
        }
        assert!(lone[0] > lone[3], "{lone:?}");
        // Landed within about 100 ms.
        assert!(
            lone.iter().rposition(|step| *step != 0.).unwrap() < 14,
            "{lone:?}"
        );
    }

    /// A spin stops soon after its last notch: within what's held of that
    /// notch and an ease.
    #[test]
    fn a_spin_stops_soon_after_its_last_notch() {
        for gap in [25, 40, 60, 100] {
            let notches: Vec<_> = (0..12).map(|i| (i * gap, 42.)).collect();
            let steps = wheel_steps(&notches, 200);
            let last = (steps.iter().rposition(|step| *step != 0.).unwrap() + 1) as f32 * 8.333;
            let after = last - (11 * gap) as f32;
            assert!(
                after <= 190.,
                "every {gap} ms, moved {after} ms after the last"
            );
        }
    }

    /// Notches apart are steps of their own: none is held back for the next.
    #[test]
    fn notches_further_apart_than_a_spin_each_start_at_once() {
        let steps = wheel_steps(&[(0, 42.), (200, 42.), (400, 42.)], 100);
        let after = |ms: f32| steps[(ms / 8.333) as usize + 1];
        for ms in [200., 400.] {
            assert!(after(ms) >= 7., "{ms}: {steps:?}");
        }
    }

    /// Turning the wheel back takes the content back at once, and it lands on
    /// the sum of what the wheel said.
    #[test]
    fn a_wheel_turned_back_goes_back_at_once() {
        let steps = wheel_steps(&[(0, 42.), (40, 42.), (80, 42.), (110, -84.)], 80);
        assert_eq!(steps.iter().sum::<f32>(), 42.);
        let back = steps.iter().position(|step| *step < 0.).unwrap();
        assert!(back as f32 * 8.333 <= 130., "{steps:?}");
    }

    #[test]
    fn out_of_phase_input_advances_evenly() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        let step = Duration::from_micros(8333);
        let mut deltas = Vec::new();
        for i in 0..60u32 {
            // Two events land in some refresh intervals and none in others.
            let jitter = if i % 3 == 0 { 3 * MS } else { Duration::ZERO };
            resampler.push_precise(start + step * i + jitter, (0., 10.));
            let refresh = start + step * i + step / 2 + 2 * MS;
            if let Some((_, dy)) = resampler.sample(refresh, Duration::from_micros(8333)) {
                deltas.push(dy);
            }
        }
        let steady = &deltas[4..deltas.len() - 4];
        for dy in steady {
            assert!((dy - 10.).abs() < 4.5, "{deltas:?}");
        }
    }

    /// Each event's motion, one frame's worth, sampled at a refresh of `refresh`
    /// just after it came: how far behind what has come each frame stands.
    fn behind(refresh: Duration, events: Duration) -> f32 {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        let (mut received, mut applied) = (0., 0.);
        for i in 0..120u32 {
            // Jitter past the half-margin's tolerance: the fixed delay alone.
            let wobble = if i % 2 == 0 {
                Duration::from_millis(1)
            } else {
                Duration::ZERO
            };
            resampler.push_precise(start + events * i + wobble, (0., 10.));
            received += 10.;
            let at = start + events * i + Duration::from_micros(2500);
            applied += resampler.sample(at, refresh).map_or(0., |(_, dy)| dy);
        }
        received - applied
    }

    /// At 60 Hz events keep up: a refresh stands a short time behind them, not
    /// a whole interval.
    #[test]
    fn a_slow_refresh_stands_a_short_time_behind() {
        let refresh = Duration::from_micros(16_667);
        let behind = behind(refresh, refresh);
        assert!(behind < 6., "{behind}");
    }

    /// At 120 Hz frames outrun events: they're bridged, as ever, a gap behind.
    #[test]
    fn a_fast_refresh_stays_behind_to_stay_even() {
        let behind = behind(Duration::from_micros(8_333), Duration::from_micros(16_667));
        assert!(behind > 5., "{behind}");
    }

    /// A slow device's events at 60 Hz are bridged too: not a jump every few frames.
    #[test]
    fn events_slower_than_a_slow_refresh_are_still_bridged() {
        let behind = behind(
            Duration::from_micros(16_667),
            3 * Duration::from_micros(16_667),
        );
        assert!(behind > 5., "{behind}");
    }

    /// Events 30 a second, slower than any refresh: a frame between two of them
    /// doesn't run out and stop. Steps stay even, at 60 Hz and 120 Hz.
    /// How far behind a stream of events `gaps` apart stands once it's going.
    fn delay_after(gaps: &[u64]) -> Duration {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        let mut at = start;
        resampler.push_precise(at, (0., 10.));
        for gap in gaps.iter().cycle().take(40) {
            at += Duration::from_millis(*gap);
            resampler.push_precise(at, (0., 10.));
        }
        resampler.sample(at, Duration::from_micros(8_333));
        resampler.delay()
    }

    #[test]
    fn only_a_steady_slow_stream_stands_far_behind() {
        // Steady at 30 a second, and wandering by a fifth: bridged as a whole.
        assert!(delay_after(&[33]) > Duration::from_millis(30));
        assert!(delay_after(&[27, 39, 33, 36, 30]) > Duration::from_millis(30));
        // A hand's: fast, then a long gap, then fast. No further than 20 ms.
        assert!(delay_after(&[8, 8, 40, 9, 50, 8]) <= STALL_BRIDGE);
    }

    #[test]
    fn events_thirty_a_second_move_every_frame_evenly() {
        for refresh in [Duration::from_micros(8_333), Duration::from_micros(16_667)] {
            let start = Instant::now();
            let mut resampler = ScrollResampler::default();
            let gap = Duration::from_micros(33_333);
            let frames = (2_000_000 / refresh.as_micros()) as u32;
            let (mut steps, mut next) = (Vec::new(), 0u32);
            for frame in 0..frames {
                let now = start + refresh * frame;
                while gap * next <= now - start {
                    resampler.push_precise(start + gap * next, (0., 10.));
                    next += 1;
                }
                steps.push(resampler.sample(now, refresh).map_or(0., |(_, dy)| dy));
            }
            let steady = &steps[steps.len() / 2..];
            let mean = steady.iter().sum::<f32>() / steady.len() as f32;
            assert!(
                steady.iter().all(|step| (step - mean).abs() < mean * 0.5),
                "{refresh:?}: {steady:?}"
            );
        }
    }

    #[test]
    fn a_stalled_stroke_applies_everything() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        resampler.push_precise(start, (0., 12.));
        resampler.push_precise(start + 8 * MS, (0., 12.));
        let first = resampler
            .sample(start + 9 * MS, Duration::from_micros(8333))
            .unwrap_or_default()
            .1;
        let rest = resampler
            .sample(start + 200 * MS, Duration::from_micros(8333))
            .unwrap_or_default()
            .1;
        assert!((first + rest - 24.).abs() < 1e-3);
        assert!(!resampler.pending());
    }

    #[test]
    fn restarting_after_a_stall_preserves_undelivered_motion() {
        for gap in [49, 50, 60, 100] {
            for delta in [(10., 20.), (-10., -20.), (-10., 20.)] {
                let start = Instant::now();
                let mut resampler = ScrollResampler::default();
                resampler.push_precise(start, delta);
                let first = resampler
                    .sample(start + 2 * MS, Duration::from_micros(8333))
                    .unwrap_or_default();
                resampler.push_precise(start + gap * MS, delta);
                let rest = resampler.flush().unwrap_or_default();
                assert!((first.0 + rest.0 - 2. * delta.0).abs() < 1e-3);
                assert!((first.1 + rest.1 - 2. * delta.1).abs() < 1e-3);
                assert!(!resampler.pending());
            }
        }
    }

    #[test]
    fn restarting_after_a_stall_delivers_the_debt_on_a_frame() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        resampler.push_precise(start, (0., 10.));
        let first = resampler
            .sample(start + 2 * MS, Duration::from_micros(8333))
            .unwrap_or_default()
            .1;
        resampler.push_precise(start + 60 * MS, (0., 10.));
        let rest = resampler
            .sample(start + 100 * MS, Duration::from_micros(8333))
            .unwrap_or_default()
            .1;
        assert!((first + rest - 20.).abs() < 1e-3);
        assert!(!resampler.pending());
    }

    #[test]
    fn notches_that_dont_ease_move_whole_on_the_next_refresh() {
        let start = Instant::now();
        let refresh = Duration::from_micros(8333);
        let mut resampler = ScrollResampler::default();
        resampler.set_wheel_eases(false);
        resampler.push_wheel(start, (0., 30.));
        assert_eq!(resampler.sample(start + refresh, refresh), Some((0., 30.)));
        assert_eq!(resampler.sample(start + refresh * 2, refresh), None);
        assert!(!resampler.pending());
        // Two within one refresh move together, on the one after.
        resampler.push_wheel(start + refresh * 3, (0., 30.));
        resampler.push_wheel(start + refresh * 3 + Duration::from_millis(4), (0., 30.));
        assert_eq!(
            resampler.sample(start + refresh * 4, refresh),
            Some((0., 60.))
        );
        assert!(!resampler.pending());
    }

    #[test]
    fn wheel_notches_ease_in_and_finish() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        resampler.push_wheel(start, (0., 84.));
        let mut total = 0.;
        let mut steps = Vec::new();
        for i in 1..=30u32 {
            if let Some((_, dy)) = resampler.sample(
                start + Duration::from_micros(8333) * i,
                Duration::from_micros(8333),
            ) {
                total += dy;
                steps.push(dy);
            }
        }
        assert!((total - 84.).abs() < 1e-3);
        assert!(steps[0] > steps[3], "{steps:?}");
        assert!(steps.iter().all(|step| step.fract() == 0.), "{steps:?}");
        assert!(!resampler.pending());
    }

    /// Eased notches applied far down long content land exactly on their
    /// destination, as f32 sums of fractional steps would not.
    #[test]
    fn wheel_notches_land_exactly_far_down() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        let mut offset = 79_952_f32;
        for notch in 0..5u32 {
            resampler.push_wheel(start + Duration::from_millis(25) * notch, (0., 56.));
        }
        for i in 1..=60u32 {
            if let Some((_, dy)) = resampler.sample(
                start + Duration::from_micros(8333) * i,
                Duration::from_micros(8333),
            ) {
                offset += dy;
            }
        }
        assert_eq!(offset, 79_952. + 280.);
        assert!(!resampler.pending());
    }

    #[test]
    fn flush_delivers_all_buffered_motion() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        resampler.push_precise(start, (3., 5.));
        resampler.push_wheel(start, (0., 28.));
        assert_eq!(resampler.flush(), Some((3., 33.)));
        assert!(!resampler.pending());
    }

    /// Exercise the real sampler at several input/display phases. Distance
    /// behind the latest received event is expressed as equivalent input time.
    fn stroke(display_hz: u32, input_hz: u32, jitter: bool) -> (f64, f64) {
        let frame = Duration::from_secs_f64(1. / f64::from(display_hz));
        let input = Duration::from_secs_f64(1. / f64::from(input_hz));
        let velocity = 1200.;
        let delta = velocity / input_hz as f32;
        let mut lags = Vec::new();
        let mut steps = Vec::new();
        for phase in 0..16 {
            let start = Instant::now();
            let mut resampler = ScrollResampler::default();
            let mut next = 0;
            let mut received = 0.;
            let mut applied = 0.;
            for index in 0..display_hz * 3 {
                let at = frame * index + frame * phase / 16;
                while next < input_hz * 3 {
                    let event = input * next
                        + if jitter && next % 3 == 0 {
                            input.mul_f32(0.3)
                        } else {
                            Duration::ZERO
                        };
                    if event > at {
                        break;
                    }
                    resampler.push_precise(start + event, (0., delta));
                    received += delta;
                    next += 1;
                }
                let step = resampler.sample(start + at, frame).unwrap_or_default().1;
                applied += step;
                assert!(step >= 0., "forward input must never move backwards");
                assert!(
                    applied <= received + 0.01,
                    "never predict past received input"
                );
                if index > display_hz / 2 {
                    lags.push(f64::from(received - applied) / f64::from(velocity) * 1000.);
                    steps.push(f64::from(step));
                }
            }
            applied += resampler.flush().unwrap_or_default().1;
            assert!((applied - received).abs() < 0.01);
        }
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        let cv = (steps.iter().map(|step| (step - mean).powi(2)).sum::<f64>() / steps.len() as f64)
            .sqrt()
            / mean;
        (lags.iter().sum::<f64>() / lags.len() as f64, cv)
    }

    #[test]
    fn regular_input_is_responsive_without_uneven_frames() {
        for (display, input, max_lag) in [(60, 60, 3.), (60, 120, 1.5), (120, 120, 1.5)] {
            let (lag, cv) = stroke(display, input, false);
            assert!(lag < max_lag, "{display}/{input}: {lag} ms behind input");
            assert!(cv < 0.001, "{display}/{input}: uneven motion {cv}");
        }
    }

    #[test]
    fn sparse_or_mismatched_input_keeps_even_frames() {
        for (display, input) in [(120, 60), (60, 100), (144, 120), (240, 60)] {
            let (_, cv) = stroke(display, input, false);
            assert!(cv < 0.001, "{display}/{input}: uneven motion {cv}");
        }
    }

    #[test]
    fn jittery_input_keeps_the_interpolation_margin() {
        let (lag, cv) = stroke(60, 60, true);
        // At a slow refresh the margin is `SLOW_REFRESH_DELAY`, short of this
        // stream's 5 ms of jitter past its 16.7 ms: less even than a whole interval
        // behind (under 0.21), for 8 ms less lag. At 120 Hz it stays a whole interval.
        assert!(lag > 2., "irregular input needs its interpolation margin");
        assert!(cv < 0.3, "jitter must stay smoothed, got {cv}");
        let (lag, cv) = stroke(120, 60, true);
        assert!(lag > 7., "irregular input needs its interpolation margin");
        assert!(cv < 0.25, "jitter must stay smoothed, got {cv}");
    }

    #[test]
    fn jitter_after_a_regular_run_never_reverses_or_loses_distance() {
        let start = Instant::now();
        let frame = Duration::from_micros(8333);
        let mut resampler = ScrollResampler::default();
        let mut received = 0.;
        let mut applied = 0.;
        for index in 0..120 {
            let jitter = if index >= 60 && index % 3 == 0 {
                3 * MS
            } else {
                Duration::ZERO
            };
            resampler.push_precise(start + frame * index + jitter, (0., 10.));
            received += 10.;
            let step = resampler
                .sample(start + frame * index + 6 * MS, frame)
                .unwrap_or_default()
                .1;
            assert!(step >= 0.);
            applied += step;
            assert!(applied <= received + 0.001);
        }
        applied += resampler.flush().unwrap_or_default().1;
        assert!((received - applied).abs() < 0.001);
    }

    #[test]
    fn reversal_after_a_regular_run_preserves_the_exact_destination() {
        let start = Instant::now();
        let frame = Duration::from_micros(8333);
        let mut resampler = ScrollResampler::default();
        let mut applied = 0.;
        for index in 0..120 {
            resampler.push_precise(
                start + frame * index,
                (0., if index < 60 { 10. } else { -10. }),
            );
            applied += resampler
                .sample(start + frame * index + 4 * MS, frame)
                .unwrap_or_default()
                .1;
        }
        applied += resampler.flush().unwrap_or_default().1;
        assert!(applied.abs() < 0.001, "reversal stranded {applied} points");
    }
}
