//! Display-paced scrolling.
//!
//! Trackpads report motion on their own clock, out of phase with the display:
//! applying each event on arrival moves content 0, 1 or 2 events' worth per
//! refresh, which reads as judder even at full frame rate. Continuous (pixel)
//! input is instead resampled: each refresh applies the motion up to one input
//! interval before the refresh, interpolated between the events around it, so
//! every frame advances evenly. At a slow refresh (60 Hz) events keep up with
//! frames, and being an interval behind costs more than it evens out: each
//! refresh takes what has come. Discrete wheel notches have no motion curve of
//! their own; they ease towards their destination over a few refreshes.
use std::{collections::VecDeque, time::Duration, time::Instant};

/// A pause this long ends a stroke: whatever is still buffered is applied.
const STALL: Duration = Duration::from_millis(50);
/// Refreshes keep running this long after the last input, so a gesture never
/// stops and restarts the display link (a lost refresh each time) when the
/// applied motion momentarily catches up with the input.
const LINGER: Duration = Duration::from_millis(120);
/// Bounds for the sampling delay, which tracks the input interval.
const MIN_DELAY: Duration = Duration::from_millis(4);
const MAX_DELAY: Duration = Duration::from_millis(20);
/// Refreshes this long or longer apply what has come, with no delay, while
/// events come within `SLOW_EVENTS` of them. A real trackpad's events through
/// this at 60 Hz: position 4.9 ms behind the read that carried it (p50; 12.9 at
/// p95) against 19 (38), steps over 50% off their neighbours 23% against 14%.
pub(crate) const SLOW_REFRESH: Duration = Duration::from_millis(12);
/// Events further apart than this many refreshes are still bridged: a slow
/// device's motion would otherwise show every few frames.
const SLOW_EVENTS: f32 = 2.;
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
    /// The time between refreshes (`set_refresh`).
    refresh: Duration,
    /// Arrival time and cumulative position of each buffered event.
    points: VecDeque<(Instant, (f32, f32))>,
    /// Cumulative continuous motion received and applied.
    received: (f32, f32),
    applied: (f32, f32),
    /// Smoothed spacing between continuous events.
    interval: Option<Duration>,
    /// Wheel distance not yet applied, and when it last advanced.
    wheel: (f32, f32),
    wheel_at: Option<Instant>,
    /// Of that, what a spin hasn't handed over yet, how much of it a second,
    /// when the last notch came, and the smoothed spacing between a spin's.
    wheel_held: (f32, f32),
    wheel_rate: (f32, f32),
    wheel_notch: Option<Instant>,
    wheel_gap: Option<Duration>,
    /// When the latest input arrived.
    last_input: Option<Instant>,
    /// Pointer position of the latest input; motion is delivered there.
    pub position: (f32, f32),
}

impl ScrollResampler {
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
                self.interval = Some(match self.interval {
                    Some(interval) => interval.mul_f32(0.7) + gap.mul_f32(0.3),
                    None => gap,
                });
            }
            _ => {
                // A new stroke: the first event's motion spans one interval
                // before it, not the whole idle gap.
                let delay = self.delay();
                self.points.clear();
                self.applied = self.received;
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
        (delta != (0., 0.)).then_some(delta)
    }
    /// The motion to apply at a display refresh processed at `now`.
    /// The time between the refreshes it's sampled at.
    pub fn set_refresh(&mut self, refresh: Duration) {
        self.refresh = refresh;
    }
    pub fn sample(&mut self, now: Instant) -> Option<(f32, f32)> {
        let target = match self.points.back() {
            Some((last, total)) if now.duration_since(*last) >= STALL => *total,
            Some(_) => self.position_at(now.checked_sub(self.delay()).unwrap_or(now)),
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
        let horizon = now.checked_sub(self.delay()).unwrap_or(now);
        while self.points.len() > 1 && self.points[1].0 <= horizon {
            self.points.pop_front();
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
        if self.refresh >= SLOW_REFRESH
            && self
                .interval
                .is_some_and(|interval| interval <= self.refresh.mul_f32(SLOW_EVENTS))
        {
            return Duration::ZERO;
        }
        self.interval
            .unwrap_or(Duration::from_millis(8))
            .clamp(MIN_DELAY, MAX_DELAY)
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
            steps.push(resampler.sample(now).unwrap_or_default().1);
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
            if let Some((_, dy)) = resampler.sample(refresh) {
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
        resampler.set_refresh(refresh);
        let (mut received, mut applied) = (0., 0.);
        for i in 0..120u32 {
            resampler.push_precise(start + events * i, (0., 10.));
            received += 10.;
            let at = start + events * i + Duration::from_micros(1500);
            applied += resampler.sample(at).map_or(0., |(_, dy)| dy);
        }
        received - applied
    }

    /// At 60 Hz events keep up: each refresh shows what has come.
    #[test]
    fn a_slow_refresh_shows_what_has_come() {
        let refresh = Duration::from_micros(16_667);
        assert_eq!(behind(refresh, refresh), 0.);
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

    #[test]
    fn a_stalled_stroke_applies_everything() {
        let start = Instant::now();
        let mut resampler = ScrollResampler::default();
        resampler.push_precise(start, (0., 12.));
        resampler.push_precise(start + 8 * MS, (0., 12.));
        let first = resampler.sample(start + 9 * MS).unwrap_or_default().1;
        let rest = resampler.sample(start + 200 * MS).unwrap_or_default().1;
        assert!((first + rest - 24.).abs() < 1e-3);
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
            if let Some((_, dy)) = resampler.sample(start + Duration::from_micros(8333) * i) {
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
            if let Some((_, dy)) = resampler.sample(start + Duration::from_micros(8333) * i) {
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
}
