//! `ui.frames`: how smoothly the app's frames came after an action, for
//! analytics, as the web's own measures count them from
//! `requestAnimationFrame`, and on the desktop from the same frame clock
//! animations tick on (each frame's expected time on screen).
//!
//! - `ui.frames.measure(fn, { window_ms = 1000 })` runs `fn` and gives a
//!   promise, settled once frames have come for `window_ms` (and at least
//!   two), of `{ start_ms, times, frames, long_frames, duration_ms,
//!   settled_ms, blocked_ms }`: `duration_ms` to the second frame,
//!   `blocked_ms` the sum of the gaps between frames over `LONG_FRAME_MS`
//!   (whole gaps, the first counted from the start), `settled_ms` the later
//!   of `duration_ms` and the end of the last such gap. A window that stops
//!   showing frames (hidden, minimised) ends it at its deadline with only
//!   `duration_ms`, if it got that far; a new measure ends the last one the
//!   same way.
//! - `ui.frames.begin()` records until `m:finish()` (a drag, a resize) or
//!   `CAP` (`capped = true`), and `finish` gives the same.
//!
//! `times` are each frame's time from the start, in milliseconds, so other
//! shapes (a gesture's percentiles, frames dropped against the refresh)
//! can be worked out from them. Nothing runs while no measure does: a
//! measure is a task awaiting frames, which keeps them coming (as a rAF
//! loop does) until it ends.
use crate::{
    calls::{self, PromiseUd},
    time::Instant,
    value::{Str, Value},
};
use mlua::{Function, Lua, Table, UserData, UserDataMethods};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::pin,
    rc::Rc,
    task::{Poll, Waker},
    time::Duration,
};

/// A gap between frames longer than this blocked the app (the web's
/// `LONG_FRAME_MS`).
const LONG_FRAME_MS: f64 = 50.;
/// How long `ui.frames.measure` records after its action, by default (the
/// web's `SWITCH_SETTLE_WINDOW_MS`).
const WINDOW_MS: f64 = 1000.;
/// How long past its window a measure waits for frames that don't come (a
/// hidden window's) before it ends.
const GRACE: Duration = Duration::from_millis(250);
/// The longest `ui.frames.begin` records.
const CAP: Duration = Duration::from_secs(30);

pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    let frames = lua.create_table()?;
    frames.set(
        "measure",
        lua.create_function(|lua, (action, options): (Function, Option<Table>)| {
            let window = match options
                .as_ref()
                .map(|o| o.get::<Option<f64>>("window_ms"))
                .transpose()?
                .flatten()
            {
                None => WINDOW_MS,
                Some(ms) if ms.is_finite() && ms > 0. => ms,
                Some(ms) => {
                    return Err(mlua::Error::runtime(format!(
                        "ui.frames.measure: `window_ms` is a number of milliseconds above 0, not {ms}"
                    )));
                }
            };
            let sample = Sample::start(lua, Until::Window(window))?;
            let ran = action.call::<()>(());
            let promise = sample.settled(lua, "ui.frames.measure")?;
            ran.map(|()| promise)
        })?,
    )?;
    frames.set(
        "begin",
        lua.create_function(|lua, ()| Ok(Measure(Sample::start(lua, Until::Finished)?)))?,
    )?;
    ui.set("frames", frames)
}

/// What ends a sample.
#[derive(Clone, Copy)]
enum Until {
    /// Frames for this many milliseconds, and two at least.
    Window(f64),
    /// `m:finish()`, or `CAP`.
    Finished,
}

/// One measure's frames, as they come.
struct Sample {
    start: Instant,
    /// The start on the wall clock (`ui.now`'s).
    start_ms: f64,
    until: Until,
    /// Each frame's time from the start, in milliseconds.
    times: RefCell<Vec<f64>>,
    /// Ended early: finished, or a newer measure began.
    ended: Cell<bool>,
    /// The recording task, waiting, to wake when it ends early.
    waker: RefCell<Option<Waker>>,
    /// Reached `CAP`.
    capped: Cell<bool>,
    /// The promise of what it measured, once it is asked for.
    promise: RefCell<Option<Rc<calls::Promise>>>,
    /// What it measured, once it has.
    measured: RefCell<Option<Value>>,
}

impl Sample {
    /// A sample from now, recording on the host's frames.
    fn start(lua: &Lua, until: Until) -> mlua::Result<Rc<Self>> {
        let env = crate::node::env(lua)?;
        let Some(frames) = env.timers.frames() else {
            return Err(mlua::Error::runtime(
                "ui.frames: there are no frames to measure until the app's view is mounted",
            ));
        };
        // From the action, on the frames' own timeline: no earlier than the
        // last frame's time (a frame in flight is due on screen a little
        // after now; a test's frames keep a time of their own).
        let now = zgx::clock::now();
        let start = frames.last().map_or(now, |last| last.time.max(now));
        let sample = Rc::new(Self {
            start,
            start_ms: zgx::clock::system_now()
                .duration_since(crate::time::SystemTime::UNIX_EPOCH)
                .map_or(0., |d| d.as_secs_f64() * 1000.),
            until,
            times: RefCell::default(),
            ended: Cell::new(false),
            waker: RefCell::default(),
            capped: Cell::new(false),
            promise: RefCell::default(),
            measured: RefCell::default(),
        });
        // A new measure ends the last, as the web's next switch does.
        let weak = Rc::downgrade(&sample);
        env.timers.measuring(Box::new(move || {
            if let Some(last) = weak.upgrade() {
                last.end();
            }
        }));
        let recording = sample.clone();
        let runtime = env.runtime.clone();
        env.timers.spawn(async move {
            recording.record(&frames).await;
            let measured = recording.measured_value();
            *recording.measured.borrow_mut() = Some(measured.clone());
            if let Some(promise) = recording.promise.take() {
                runtime.batch(|| calls::resolve(&promise, measured));
            }
        });
        Ok(sample)
    }

    /// End it now, with what it has.
    fn end(&self) {
        self.ended.set(true);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    /// A promise of what it measures, `name` its maker.
    fn settled(self: &Rc<Self>, lua: &Lua, name: &str) -> mlua::Result<PromiseUd> {
        let promise = calls::pending(lua, name.into())?;
        match self.measured.borrow().clone() {
            Some(measured) => calls::resolve(&promise, measured),
            None => *self.promise.borrow_mut() = Some(promise.clone()),
        }
        Ok(PromiseUd(promise))
    }

    /// Record each frame's time until it ends: its window covered (two
    /// frames at least), ended early, or past its deadline with no frame.
    async fn record(&self, frames: &zgx::frame::FrameClock) {
        let deadline = match self.until {
            Until::Window(ms) => Duration::from_secs_f64(ms / 1000.) + GRACE,
            Until::Finished => CAP,
        };
        let sleep = zgx::timer::sleep_until(self.start + deadline);
        let mut sleep = pin!(sleep);
        loop {
            let next = frames.next();
            let mut next = pin!(next);
            let frame = std::future::poll_fn(|cx| {
                if self.ended.get() {
                    return Poll::Ready(None);
                }
                if let Poll::Ready(frame) = next.as_mut().poll(cx) {
                    return Poll::Ready(Some(frame));
                }
                if sleep.as_mut().poll(cx).is_ready() {
                    if matches!(self.until, Until::Finished) {
                        self.capped.set(true);
                    }
                    return Poll::Ready(None);
                }
                *self.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            })
            .await;
            let Some(frame) = frame else {
                return;
            };
            let at = frame
                .time
                .saturating_duration_since(self.start)
                .as_secs_f64()
                * 1000.;
            let mut times = self.times.borrow_mut();
            times.push(at);
            if let Until::Window(ms) = self.until
                && times.len() >= 2
                && at >= ms
            {
                return;
            }
        }
    }

    /// What it measured, by the web's rules (`frames::measured`).
    fn measured_value(&self) -> Value {
        // A gesture's measure covers what it was asked to, however it ended.
        let covered = match self.until {
            Until::Window(ms) => self.times.borrow().last().is_some_and(|at| *at >= ms),
            Until::Finished => true,
        };
        let m = measured(&self.times.borrow(), covered);
        let mut out: Vec<(Str, Value)> = vec![
            ("start_ms".into(), Value::Num(self.start_ms)),
            (
                "times".into(),
                Value::List(self.times.borrow().iter().map(|t| Value::Num(*t)).collect()),
            ),
            ("frames".into(), Value::Num(m.frames as f64)),
            ("long_frames".into(), Value::Num(m.long_frames as f64)),
        ];
        if let Some(duration) = m.duration_ms {
            out.push(("duration_ms".into(), Value::Num(duration)));
        }
        if let Some(settled) = m.settled_ms {
            out.push(("settled_ms".into(), Value::Num(settled)));
        }
        if let Some(blocked) = m.blocked_ms {
            out.push(("blocked_ms".into(), Value::Num(blocked)));
        }
        if self.capped.get() {
            out.push(("capped".into(), Value::Bool(true)));
        }
        Value::Map(out.into())
    }
}

/// What a sample's frames come to.
#[derive(Debug, PartialEq)]
pub(crate) struct Measured {
    pub frames: usize,
    pub long_frames: usize,
    pub duration_ms: Option<f64>,
    pub settled_ms: Option<f64>,
    pub blocked_ms: Option<f64>,
}

/// The web's tab-switch measure (`measureSwitch`) of frames at `times`
/// (milliseconds from the start), `covered` if they came for its whole
/// window: `duration_ms` the second frame's time; `blocked_ms` the sum of
/// the gaps over `LONG_FRAME_MS`, the first from the start; `settled_ms`
/// the later of `duration_ms` and the last such gap's end; each rounded.
/// A sample that ended before its window (hidden) has `duration_ms` alone,
/// if it got two frames.
pub(crate) fn measured(times: &[f64], covered: bool) -> Measured {
    let (mut last, mut blocked, mut settled_at, mut long_frames) = (0., 0., 0_f64, 0);
    for &at in times {
        let gap = at - last;
        if gap > LONG_FRAME_MS {
            blocked += gap;
            settled_at = at;
            long_frames += 1;
        }
        last = at;
    }
    let duration = times.get(1).map(|at| at.round());
    let whole = covered && duration.is_some();
    Measured {
        frames: times.len(),
        long_frames,
        duration_ms: duration,
        settled_ms: duration
            .filter(|_| whole)
            .map(|d| d.max(settled_at).round()),
        blocked_ms: whole.then(|| blocked.round()),
    }
}

/// `ui.frames.begin()`'s measure: `m:finish()` ends it.
struct Measure(Rc<Sample>);
impl UserData for Measure {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("finish", |lua, m, ()| {
            m.0.end();
            m.0.settled(lua, "m:finish")
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_smooth_switch_settles_at_its_second_frame() {
        let times: Vec<f64> = (1..=60).map(|i| f64::from(i) * 16.7).collect();
        assert_eq!(
            measured(&times, true),
            Measured {
                frames: 60,
                long_frames: 0,
                duration_ms: Some(33.),
                settled_ms: Some(33.),
                blocked_ms: Some(0.),
            }
        );
    }

    #[test]
    fn whole_gaps_over_50_ms_block_and_the_last_one_settles() {
        // 120 ms before the first frame (the action's own work), then a
        // 70 ms gap, then smooth: blocked 190, settled at the gap's end.
        let mut times = vec![120., 136.7, 206.7];
        times.extend((1..60).map(|i| 206.7 + f64::from(i) * 16.7));
        let m = measured(&times, true);
        assert_eq!(m.long_frames, 2);
        assert_eq!(m.duration_ms, Some(137.));
        assert_eq!(m.blocked_ms, Some(190.));
        assert_eq!(m.settled_ms, Some(207.));
    }

    #[test]
    fn a_sample_cut_short_gives_its_duration_alone() {
        let m = measured(&[16.7, 33.4, 50.1], false);
        assert_eq!(m.duration_ms, Some(33.));
        assert_eq!((m.settled_ms, m.blocked_ms), (None, None));
        let none = measured(&[16.7], false);
        assert_eq!((none.frames, none.duration_ms), (1, None));
    }
}
