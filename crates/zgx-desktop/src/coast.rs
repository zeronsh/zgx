//! A trackpad scroll's momentum on macOS, as AppKit's scroll views make it:
//! from the fingers' speed as they lift, on a curve of its own, not the
//! system's momentum events (an `NSScrollView` ignores those: a stream of
//! them posted after a lift left its coast as it was, measured).
//!
//! The curve is a quartic ease-out that stops in a finite time: from speed
//! `v` it goes `K (v / 4K)^(4/3)` in `(v / 4K)^(1/3)` seconds, `K` points a
//! second to the fourth, its speed `4K (T - t)^3` at `t`. Both axes travel
//! one straight line. Each refresh shows where it is when that refresh
//! reaches the screen, in whole points.
//!
//! Measured on an `NSScrollView` with real system scroll events (M5 Max):
//! fitted `K` 1030-1110 over flicks of 1000 to 6000 points a second, every
//! step a whole point; the same with its constant set to 2000 fitted 2164.
use std::time::{Duration, Instant};

/// The curve's constant, in points a second to the fourth (AppKit's default).
const K: f32 = 1000.;
/// The longest a coast runs: one longer ends where it would be by then, on
/// a curve steepened to stop there.
const LONGEST: f32 = 2.;
/// How much a new speed counts against the speed so far, and the shortest
/// and longest time between the fingers' scrolls it counts: as AppKit
/// measures their speed (see `zgx::elastic`'s, the same).
const WEIGHT: f32 = 0.8;
const SOONEST: f32 = 0.002;
const LATEST: f32 = 0.2;

/// The fingers' speed along one axis.
#[derive(Clone, Copy, Debug, Default)]
struct Speed {
    speed: f32,
    measured: bool,
    pending: f32,
}

/// The fingers' speed while they scroll, then the coast after they lift.
#[derive(Debug, Default)]
pub(crate) struct Coast {
    speed: [Speed; 2],
    /// When the fingers last scrolled (or touched down).
    at: Option<Instant>,
    /// The coast under way: when it started, its constant and length, the
    /// distance it goes along either axis, and the whole points shown so far.
    running: Option<Running>,
}

#[derive(Clone, Copy, Debug)]
struct Running {
    start: Instant,
    k: f32,
    length: f32,
    distance: (f32, f32),
    shown: (f32, f32),
}

impl Coast {
    /// Fingers touched down: what was coasting stops.
    pub(crate) fn touched(&mut self, now: Instant) {
        *self = Self {
            at: Some(now),
            ..Self::default()
        };
    }

    /// The fingers scrolled `delta` (points, either axis) at `at`.
    pub(crate) fn scrolled(&mut self, delta: (f32, f32), at: Instant) {
        let Some(last) = self.at else {
            self.touched(at);
            return;
        };
        let dt = at.saturating_duration_since(last).as_secs_f32();
        let soon = dt < SOONEST;
        for (speed, delta) in self.speed.iter_mut().zip([delta.0, delta.1]) {
            if !delta.is_finite() {
                continue;
            }
            if soon {
                speed.pending += delta;
                continue;
            }
            let delta = delta + std::mem::take(&mut speed.pending);
            let now = delta / dt;
            speed.speed = if !speed.measured || dt > LATEST || now * speed.speed < 0. {
                now
            } else {
                WEIGHT * now + (1. - WEIGHT) * speed.speed
            };
            speed.measured = true;
        }
        if !soon {
            self.at = Some(at);
        }
    }

    /// The fingers lifted at `now`: the coast starts, if they were moving.
    /// Whether it did.
    pub(crate) fn lifted(&mut self, now: Instant) -> bool {
        let rested = self
            .at
            .is_none_or(|at| now.saturating_duration_since(at).as_secs_f32() > LATEST);
        let [x, y] = self
            .speed
            .map(|speed| if speed.measured { speed.speed } else { 0. });
        self.speed = Default::default();
        self.at = None;
        if rested || (x == 0. && y == 0.) {
            self.running = None;
            return false;
        }
        // Along each axis, the distance its speed alone would go; both
        // together on one line, in the time the whole distance takes.
        let along = |v: f32| (v.abs() / (4. * K)).powf(4. / 3.) * K * v.signum();
        let distance = (along(x), along(y));
        let total = distance.0.hypot(distance.1);
        let mut length = (total / K).powf(0.25);
        let mut k = K;
        let mut distance = distance;
        if length > LONGEST {
            // Where it would be by then, reached on a steeper curve.
            let reached = (total - K * 0f32.max(length - LONGEST).powi(4)) / total;
            distance = (distance.0 * reached, distance.1 * reached);
            length = LONGEST;
            k = total * reached / LONGEST.powi(4);
        }
        zgx_gpu::present_log::log(|| {
            format!(
                "coast-start speed={x:.0},{y:.0} distance={:.0},{:.0} ms={:.0}",
                distance.0,
                distance.1,
                length * 1000.
            )
        });
        self.running = (total > 0.).then_some(Running {
            start: now,
            k,
            length,
            distance,
            shown: (0., 0.),
        });
        self.running.is_some()
    }

    /// Whether a coast runs.
    pub(crate) fn active(&self) -> bool {
        self.running.is_some()
    }

    /// Stop a coast where it is (scrolled into an edge, or replaced).
    pub(crate) fn stop(&mut self) {
        self.running = None;
    }

    /// The whole points to move for a refresh reaching the screen at
    /// `shows`, and whether that's the coast's end (then it stops).
    pub(crate) fn sample(&mut self, shows: Instant) -> Option<((f32, f32), bool)> {
        let running = self.running.as_mut()?;
        let t = shows.saturating_duration_since(running.start).as_secs_f32();
        let left = (running.length - t).max(0.);
        let total = running.distance.0.hypot(running.distance.1);
        let part = ((total - running.k * left.powi(4)) / total).clamp(0., 1.);
        let at = (
            whole(running.distance.0 * part),
            whole(running.distance.1 * part),
        );
        let delta = (at.0 - running.shown.0, at.1 - running.shown.1);
        running.shown = at;
        let done = left == 0.;
        if done {
            self.running = None;
        }
        Some((delta, done))
    }

    /// When the coast ends, if one runs.
    pub(crate) fn ends(&self) -> Option<Instant> {
        self.running
            .map(|running| running.start + Duration::from_secs_f32(running.length))
    }
}

/// While fingers are on the trackpad, AppKit hands the main thread every
/// scroll event, not those it took late merged into one: the fingers' speed
/// is measured over each, as an `NSScrollView` measures it on a thread of
/// its own. Merged, a stream at 120 Hz came two by two every 16.7 ms, and its
/// speed missed AppKit's by up to half in 3 of 9 flicks; separate, it matched
/// to 30 points a second in 5 of 6. As the fingers lift, back as it was.
pub(crate) fn fingers_down(down: bool) {
    use objc2_app_kit::NSEvent;
    use std::sync::atomic::{AtomicU8, Ordering};
    // 0: as it was; 1: off for the fingers, on before; 2: off, off before.
    static HELD: AtomicU8 = AtomicU8::new(0);
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    match (down, HELD.load(Ordering::Relaxed)) {
        (true, 0) => {
            let was = NSEvent::isMouseCoalescingEnabled();
            NSEvent::setMouseCoalescingEnabled(false);
            HELD.store(if was { 1 } else { 2 }, Ordering::Relaxed);
        }
        (false, held @ (1 | 2)) => {
            NSEvent::setMouseCoalescingEnabled(held == 1);
            HELD.store(0, Ordering::Relaxed);
        }
        _ => {}
    }
}

/// When the scroll event being handled happened, as the system stamped it:
/// AppKit measures the fingers' speed by these. Events the main thread
/// takes late come together, so when they arrive says little (a 120 Hz
/// stream taken every 16.7 ms read half its speed on its last event). Now,
/// if the event isn't a scroll's.
pub(crate) fn event_time() -> Instant {
    use objc2_app_kit::{NSApplication, NSEventType};
    let now = Instant::now();
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return now;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return now;
    };
    if event.r#type() != NSEventType::ScrollWheel {
        return now;
    }
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }
    // The system's uptime (`CLOCK_UPTIME_RAW`), which event times count.
    let uptime = unsafe { clock_gettime_nsec_np(8) } as f64 / 1e9;
    let age = uptime - event.timestamp();
    match age.is_finite() && (0. ..1.).contains(&age) {
        true => now - Duration::from_secs_f64(age),
        false => now,
    }
}

/// A distance in whole points, as AppKit rounds its scroll views': within an
/// eighth of a whole point, that point; otherwise the nearest, halves toward
/// zero.
fn whole(distance: f32) -> f32 {
    let near = distance.round();
    if (distance - near).abs() <= 0.125 {
        return near;
    }
    let toward = distance.trunc();
    if (distance - toward).abs() == 0.5 {
        toward
    } else {
        near
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fingers scrolling `speed` points a second downward for 300 ms at 120 Hz.
    fn flicked(speed: f32, start: Instant) -> (Coast, Instant) {
        let mut coast = Coast::default();
        coast.touched(start);
        let step = Duration::from_nanos(8_333_333);
        let mut at = start;
        for _ in 0..36 {
            at += step;
            coast.scrolled((0., speed * step.as_secs_f32()), at);
        }
        (coast, at)
    }

    fn travel(coast: &mut Coast, from: Instant) -> (f32, Vec<f32>) {
        let step = Duration::from_nanos(8_333_333);
        let mut at = from;
        let (mut total, mut steps) = (0., Vec::new());
        while let Some(((_, dy), done)) = coast.sample(at) {
            total += dy;
            steps.push(dy);
            if done {
                break;
            }
            at += step;
        }
        (total, steps)
    }

    #[test]
    fn a_flick_coasts_on_appkits_curve_in_whole_points() {
        let start = Instant::now();
        // From 1000, 3000 and 6000 points a second: K (v / 4K)^(4/3) points
        // in (v / 4K)^(1/3) seconds.
        for (speed, distance, seconds) in [
            (1000., 157., 0.63),
            (3000., 681., 0.909),
            (6000., 1717., 1.145),
        ] {
            let (mut coast, lift) = flicked(speed, start);
            assert!(coast.lifted(lift));
            let ends = coast.ends().unwrap().duration_since(lift).as_secs_f32();
            assert!((ends - seconds).abs() < 0.01, "{speed}: {ends} s");
            let (total, steps) = travel(&mut coast, lift);
            assert!((total - distance).abs() <= 1., "{speed}: {total} points");
            assert!(
                steps.iter().all(|step| step.fract() == 0.),
                "whole points: {steps:?}"
            );
            // Slowing all the way: no step longer than the one before.
            assert!(
                steps.windows(2).skip(1).all(|w| w[1] <= w[0] + 1.),
                "{steps:?}"
            );
            assert!(!coast.active());
        }
    }

    #[test]
    fn a_coast_ends_by_two_seconds_and_rested_fingers_lift_without_one() {
        let start = Instant::now();
        // Over 32000 points a second, the curve runs past 2 s.
        let (mut coast, lift) = flicked(40_000., start);
        assert!(coast.lifted(lift));
        let ends = coast.ends().unwrap().duration_since(lift).as_secs_f32();
        assert!((ends - LONGEST).abs() < 1e-3, "{ends}");
        let (total, _) = travel(&mut coast, lift);
        // Where 40000 points a second would be at 2 s on the unbounded curve.
        let length = (40_000_f32 / 4000.).powf(1. / 3.);
        let full = K * length.powi(4);
        let at_two = full - K * (length - 2.).powi(4);
        assert!((total - at_two).abs() <= 1., "{total} against {at_two}");

        let (mut coast, lift) = flicked(3000., start);
        assert!(
            !coast.lifted(lift + Duration::from_millis(250)),
            "rested 250 ms"
        );
        assert!(coast.sample(lift).is_none());
    }

    #[test]
    fn touching_down_stops_a_coast() {
        let start = Instant::now();
        let (mut coast, lift) = flicked(3000., start);
        assert!(coast.lifted(lift));
        assert!(coast.sample(lift + Duration::from_millis(100)).is_some());
        coast.touched(lift + Duration::from_millis(120));
        assert!(!coast.active());
        assert!(coast.sample(lift + Duration::from_millis(130)).is_none());
    }

    #[test]
    fn whole_points_round_as_appkit_does() {
        assert_eq!(whole(2.1), 2.);
        assert_eq!(whole(2.4), 2.);
        assert_eq!(whole(2.5), 2., "halves toward zero");
        assert_eq!(whole(-2.5), -2.);
        assert_eq!(whole(2.6), 3.);
        assert_eq!(whole(2.88), 3., "within an eighth");
    }
}
