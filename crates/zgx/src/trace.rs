//! Why things ran, for devtools (the `trace` feature): per frame, which
//! signals were written and which effects that queued ran, how long each
//! took, and what each is (a node's binding, a script's effect).
//!
//! Built without the feature, none of this exists. Built with it, it does
//! nothing until [`set_enabled`]: a relaxed load on each write and run, no
//! allocation and no bookkeeping.
use crate::time::Instant;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

static ON: AtomicBool = AtomicBool::new(false);

/// Frames kept, the newest last: four seconds at 120 Hz (a fling's), until
/// [`set_kept`] says otherwise.
static KEPT: AtomicUsize = AtomicUsize::new(DEFAULT_KEPT);
/// Frames kept unless said otherwise.
pub const DEFAULT_KEPT: usize = 480;
/// The most frames kept: about three minutes at 120 Hz.
pub const MOST_KEPT: usize = 20_000;

/// Whether runs are being traced.
#[inline(always)]
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}
/// Keep the last `frames` frames (at least one, at most [`MOST_KEPT`]): a
/// few flings outrun the default. Older ones go at once.
pub fn set_kept(frames: usize) {
    let frames = frames.clamp(1, MOST_KEPT);
    KEPT.store(frames, Ordering::Relaxed);
    LOG.with_borrow_mut(|log| {
        while log.frames.len() > frames {
            log.frames.pop_front();
        }
    });
}
/// How many frames are kept ([`set_kept`]).
pub fn kept() -> usize {
    KEPT.load(Ordering::Relaxed)
}
/// Start or stop tracing (on this thread's runtimes), phases measured with
/// it (`zgx::profile::set_recording`). Off, what was recorded is dropped.
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
    crate::profile::set_recording(on);
    if !on {
        LOG.with_borrow_mut(|log| *log = Log::default());
    }
}

/// An effect's run in a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The effect's number.
    pub effect: u64,
    /// What it is, if its maker said (`name`): `@12 binding`, a script line.
    pub name: Option<Rc<str>>,
    /// The signal whose write queued it (its address, `Signal::addr`); none
    /// for a first run, or one queued otherwise.
    pub cause: Option<usize>,
    /// How long it took, in microseconds (what it ran inside included).
    pub micros: f32,
}
/// One frame's work: writes and the runs they caused, in order, and what
/// each phase of it took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Its number, counting frames traced.
    pub index: u64,
    /// Signals written (their addresses) and how many times each.
    pub writes: Vec<(usize, u32)>,
    /// Where the signals written were made, those whose maker said
    /// ([`name_signal`]: a script's line and what made it): kept as they
    /// were written, the signals may be gone by the time the frame is read.
    pub sites: Vec<(usize, Rc<str>)>,
    /// Effects run, in the order they ran.
    pub runs: Vec<Run>,
    /// Each phase that ran (`zgx::profile`'s: input, script, update, style,
    /// layout, text, damage, paint, encode, submit, present): its name,
    /// instructions (0 where not counted) and microseconds.
    pub phases: Vec<(&'static str, u64, f32)>,
    /// Not a frame: what a window did in the time frames left over
    /// (`zgx::spare`). Nothing waited for it, and it is shown as nothing.
    pub spare: bool,
    /// When it ended (was shown), in milliseconds since tracing began
    /// ([`now_ms`]'s clock): the time between frames is their pacing.
    pub ended_ms: f64,
    /// Why it did what it did, where the engine said ([`note`]): a scroll
    /// repainted rather than copied, a wait for a later refresh, input that
    /// waited for the window to present again.
    pub notes: Vec<Rc<str>>,
    /// How long the GPU itself ran the frame, in microseconds, where the
    /// renderer measures it ([`gpu`]; macOS's direct Metal frames).
    pub gpu_micros: Option<f32>,
    /// Input events dispatched in it ([`input`]): a pointer's moves, keys,
    /// wheel steps. What an interaction costs per event reads from these.
    pub events: u32,
    /// Nodes layout measured afresh in it (not taken from a measurement it
    /// kept), and those whose own measuring took longest ([`Measured`]).
    pub layout: Layout,
}

/// A frame's layout work: how many nodes were measured afresh, their time,
/// and the costliest of them by their own time (their children's apart).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// How many nodes were measured afresh.
    pub measured: u32,
    /// The outermost measures' time together, in microseconds.
    pub micros: f32,
    /// The costliest by their own time, the costliest first ([`TOP`]).
    pub top: Vec<Measured>,
}
/// A node measured afresh (`Scene::measured` without a kept result).
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    /// Its number (`NodeId::number`).
    pub node: u32,
    /// Its measuring's time, what it measured inside it included, in
    /// microseconds.
    pub micros: f32,
    /// Its own part of that: less its children's measuring.
    pub self_micros: f32,
}
/// How many of a frame's measures it keeps by their own time.
pub const TOP: usize = 12;

/// A measure in progress: when it began, and its children's time so far.
struct Open {
    began: Instant,
    children: f32,
}

#[derive(Default)]
struct Log {
    /// The measures in progress, outermost first.
    measuring: Vec<Open>,
    /// When tracing began: frames' times count from it.
    began: Option<Instant>,
    index: u64,
    current: Frame,
    /// The frame under way while spare time runs (`spare_begins`).
    held: Option<Frame>,
    frames: VecDeque<Frame>,
    causes: HashMap<u64, usize>,
    names: HashMap<u64, Rc<str>>,
    /// Signals' makers, by address ([`name_signal`]), until they drop.
    sites: HashMap<usize, Rc<str>>,
    /// The Rust lines that made signals, by address, until they drop: what
    /// a write no maker named says (`Runtime::signal`'s caller).
    made: HashMap<usize, &'static std::panic::Location<'static>>,
    /// Those lines' texts, one each.
    made_text: HashMap<(&'static str, u32), Rc<str>>,
    /// The names of effects by the Rust line that made them, one a line
    /// (`zgx/src/compose.rs:812 effect`).
    effect_text: HashMap<(&'static str, u32), Rc<str>>,
}
thread_local! {
    static LOG: RefCell<Log> = RefCell::default();
}

/// The frame log's lines of a memory report: its frames, and the names of
/// the effects it can tell apart.
pub fn memory(report: &mut crate::memory::Report) {
    LOG.with_borrow(|log| {
        // A name or note is counted once, however many frames' runs hold it
        // (a run keeps its effect's name after the effect is gone): its text
        // and its `Rc`'s counts.
        let mut seen = std::collections::HashSet::new();
        let mut text = |s: &Rc<str>| {
            if seen.insert(Rc::as_ptr(s).cast::<u8>()) {
                s.len() + 2 * size_of::<usize>()
            } else {
                0
            }
        };
        let frames = log.frames.iter().chain(&log.held).chain([&log.current]);
        let mut held = 0;
        for f in frames.clone() {
            held += size_of::<Frame>()
                + f.writes.capacity() * size_of::<(usize, u32)>()
                + f.sites.capacity() * size_of::<(usize, Rc<str>)>()
                + f.runs.capacity() * size_of::<Run>()
                + f.phases.capacity() * size_of::<(&'static str, u64, f32)>()
                + f.notes.capacity() * size_of::<Rc<str>>()
                + f.layout.top.capacity() * size_of::<Measured>()
                + f.notes.iter().map(&mut text).sum::<usize>();
        }
        report.add("devtools", "frame log", Some(log.frames.len()), Some(held));
        let table = |entries: usize, value: usize| entries * (size_of::<u64>() + value + 1);
        let mut names = table(log.names.capacity(), size_of::<Rc<str>>())
            + table(log.causes.capacity(), size_of::<usize>());
        names += log.names.values().map(&mut text).sum::<usize>();
        names += table(log.sites.capacity(), size_of::<Rc<str>>())
            + log.sites.values().map(&mut text).sum::<usize>();
        names += table(log.made.capacity(), size_of::<usize>())
            + table(log.made_text.capacity(), size_of::<Rc<str>>())
            + log.made_text.values().map(&mut text).sum::<usize>()
            + table(log.effect_text.capacity(), size_of::<Rc<str>>())
            + log.effect_text.values().map(&mut text).sum::<usize>();
        for f in frames {
            names += f
                .runs
                .iter()
                .filter_map(|r| r.name.as_ref())
                .map(&mut text)
                .sum::<usize>();
        }
        report.add(
            "devtools",
            "effect names",
            Some(log.names.len()),
            Some(names),
        );
    });
}

/// Name effect `effect` for traces (`@12 binding`, `app.luau:30 effect`).
pub fn name(effect: u64, name: impl Into<Rc<str>>) {
    if enabled() {
        LOG.with_borrow_mut(|log| {
            log.names.insert(effect, name.into());
        });
    }
}
/// Effect `effect` was made at `at` (`Runtime::effect`'s caller): traces
/// name it by that line (`zgx/src/compose.rs:812 effect`) until its maker
/// names it ([`name`]).
pub(crate) fn made_effect(effect: u64, at: &'static std::panic::Location<'static>) {
    LOG.with_borrow_mut(|log| {
        let name = log
            .effect_text
            .entry((at.file(), at.line()))
            .or_insert_with(|| Rc::from(format!("{} effect", rust_line(at))))
            .clone();
        log.names.insert(effect, name);
    });
}
/// Effect `effect` is gone: its name, and the cause of a run it was queued
/// for and never made, go too (each mount names its effects anew).
pub(crate) fn forget(effect: u64) {
    // Dropped as the thread ends, the log may be gone already.
    let _ = LOG.try_with(|log| {
        if let Ok(mut log) = log.try_borrow_mut() {
            log.names.remove(&effect);
            log.causes.remove(&effect);
        }
    });
}
/// Name signal `signal` (its address, `Signal::addr`) for traces by where
/// it was made (`app.luau:30 use_state`): a frame's writes say it, not
/// "unnamed". Forgotten as the signal drops.
pub fn name_signal(signal: usize, site: Rc<str>) {
    if enabled() {
        LOG.with_borrow_mut(|log| {
            log.sites.insert(signal, site);
        });
    }
}
/// Signal `signal` was made at `at` (`Runtime::signal`'s caller): its
/// writes say so until a maker names it ([`name_signal`]).
pub(crate) fn made(signal: usize, at: &'static std::panic::Location<'static>) {
    LOG.with_borrow_mut(|log| {
        log.made.insert(signal, at);
    });
}
/// Signal `signal` dropped: its address may be another's next.
pub(crate) fn forget_signal(signal: usize) {
    // Dropped as the thread ends, the log may be gone already.
    let _ = LOG.try_with(|log| {
        if let Ok(mut log) = log.try_borrow_mut() {
            log.sites.remove(&signal);
            log.made.remove(&signal);
        }
    });
}
/// A Rust line as a frame's writes show it: its crate's path and line
/// (`zgx-script/src/lower.rs:8290`).
fn rust_line(at: &std::panic::Location<'_>) -> Rc<str> {
    let parts: Vec<&str> = at.file().split(['/', '\\']).collect();
    let path = parts[parts.len().saturating_sub(3)..].join("/");
    Rc::from(format!("{path}:{}", at.line()))
}
/// Signal `signal` was written, queueing `queued`.
pub(crate) fn wrote(signal: usize, queued: &[u64]) {
    LOG.with_borrow_mut(|log| {
        match log.current.writes.iter_mut().find(|(s, _)| *s == signal) {
            Some((_, n)) => *n += 1,
            None => {
                log.current.writes.push((signal, 1));
                let site = match log.sites.get(&signal) {
                    Some(site) => Some(site.clone()),
                    None => log.made.get(&signal).copied().map(|at| {
                        log.made_text
                            .entry((at.file(), at.line()))
                            .or_insert_with(|| rust_line(at))
                            .clone()
                    }),
                };
                if let Some(site) = site {
                    log.current.sites.push((signal, site));
                }
            }
        }
        for id in queued {
            log.causes.entry(*id).or_insert(signal);
        }
    });
}
/// Effect `effect` ran, from `started` until now.
pub(crate) fn ran(effect: u64, started: Instant) {
    let micros = started.elapsed().as_secs_f32() * 1e6;
    LOG.with_borrow_mut(|log| {
        let cause = log.causes.remove(&effect);
        let name = log.names.get(&effect).cloned();
        log.current.runs.push(Run {
            effect,
            name,
            cause,
            micros,
        });
    });
}
/// Why the frame being traced did something (`zgx::profile::note`, which
/// builds the text only while tracing). A note given twice in a frame is kept
/// once.
pub fn note(why: String) {
    LOG.with_borrow_mut(|log| {
        if !log.current.notes.iter().any(|n| **n == *why) {
            log.current.notes.push(why.into());
        }
    });
}
/// The GPU's own time for the frame being traced (`zgx::profile::gpu_time`).
pub fn gpu(time: std::time::Duration) {
    LOG.with_borrow_mut(|log| {
        log.current.gpu_micros = Some(time.as_secs_f32() * 1e6);
    });
}
/// An input event was dispatched (`Ui::dispatch`): counted in the frame
/// being traced.
#[inline]
pub fn input() {
    if enabled() {
        counted_input();
    }
}
#[cold]
#[inline(never)]
fn counted_input() {
    LOG.with_borrow_mut(|log| log.current.events += 1);
}
/// A node begins to be measured afresh (`zgx::profile::layout_span`).
pub fn measure_begin() {
    LOG.with_borrow_mut(|log| {
        log.measuring.push(Open {
            began: Instant::now(),
            children: 0.,
        })
    });
}
/// Node `node`'s fresh measure is done: counted, timed, and kept if it is
/// among the frame's costliest by its own time.
pub fn measure_end(node: u32) {
    LOG.with_borrow_mut(|log| {
        let Some(open) = log.measuring.pop() else {
            return;
        };
        let micros = open.began.elapsed().as_secs_f32() * 1e6;
        let self_micros = (micros - open.children).max(0.);
        match log.measuring.last_mut() {
            Some(parent) => parent.children += micros,
            None => log.current.layout.micros += micros,
        }
        let layout = &mut log.current.layout;
        layout.measured += 1;
        let at = layout
            .top
            .partition_point(|kept| kept.self_micros >= self_micros);
        if at < TOP {
            layout.top.insert(
                at,
                Measured {
                    node,
                    micros,
                    self_micros,
                },
            );
            layout.top.truncate(TOP);
        }
    });
}
/// The frame being traced is done (`zgx::profile::frame`, as a renderer
/// presents it), its phases' split given: kept if anything happened in it.
pub fn end_frame(phases: Vec<(&'static str, u64, f32)>) {
    if !enabled() {
        return;
    }
    LOG.with_borrow_mut(|log| {
        log.index += 1;
        if log.current.writes.is_empty()
            && log.current.runs.is_empty()
            && log.current.notes.is_empty()
            && log.current.events == 0
            && log.current.layout.measured == 0
            && phases.is_empty()
        {
            return;
        }
        let mut frame = std::mem::take(&mut log.current);
        frame.phases = phases;
        frame.index = log.index;
        let began = *log.began.get_or_insert_with(Instant::now);
        frame.ended_ms = began.elapsed().as_secs_f64() * 1e3;
        log.frames.push_back(frame);
        while log.frames.len() > kept() {
            log.frames.pop_front();
        }
    });
}
/// Spare time begins (`zgx::profile::spare_time`): the frame under way waits
/// for it, what it has so far its own.
pub fn spare_begins() {
    LOG.with_borrow_mut(|log| log.held = Some(std::mem::take(&mut log.current)));
}
/// Spare time ended: an entry of its own, if it did anything, and the frame
/// goes on.
pub fn spare_ends(phases: Vec<(&'static str, u64, f32)>) {
    LOG.with_borrow_mut(|log| {
        let held = log.held.take().unwrap_or_default();
        if phases.is_empty() && log.current.runs.is_empty() {
            log.current = held;
            return;
        }
        log.index += 1;
        let mut frame = std::mem::replace(&mut log.current, held);
        frame.phases = phases;
        frame.index = log.index;
        frame.spare = true;
        let began = *log.began.get_or_insert_with(Instant::now);
        frame.ended_ms = began.elapsed().as_secs_f64() * 1e3;
        log.frames.push_back(frame);
        while log.frames.len() > kept() {
            log.frames.pop_front();
        }
    });
}
/// Now, on frames' clock (`Frame::ended_ms`): milliseconds since tracing
/// began.
pub fn now_ms() -> f64 {
    LOG.with_borrow_mut(|log| {
        log.began
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_secs_f64()
            * 1e3
    })
}
/// The number of the last frame traced (each frame's `index` counts from
/// here): a mark to read frames after.
pub fn frame_index() -> u64 {
    LOG.with_borrow(|log| log.index)
}
/// The last `n` frames that did anything, oldest first.
pub fn frames(n: usize) -> Vec<Frame> {
    LOG.with_borrow(|log| {
        let skip = log.frames.len().saturating_sub(n);
        log.frames.iter().skip(skip).cloned().collect()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::{compose::prelude::*, widgets::Ui};

    /// A memory report's devtools lines count a name once, wherever it's
    /// held (the names, and runs of frames kept), and one a gone effect's
    /// run alone still holds.
    #[test]
    fn a_report_counts_each_name_once_and_those_runs_keep() {
        use std::rc::Rc;
        let (named, gone): (Rc<str>, Rc<str>) = ("app.luau:30 effect".into(), "@12 binding".into());
        super::LOG.with_borrow_mut(|log| {
            *log = super::Log::default();
            log.names.insert(1, named.clone());
            let run = |name: &Rc<str>| super::Run {
                effect: 1,
                name: Some(name.clone()),
                cause: None,
                micros: 1.,
            };
            let frame = super::Frame {
                runs: vec![run(&named), run(&named), run(&gone)],
                ..Default::default()
            };
            log.frames.push_back(frame);
        });
        let mut report = crate::memory::Report::default();
        super::memory(&mut report);
        let (names, causes) =
            super::LOG.with_borrow(|log| (log.names.capacity(), log.causes.capacity()));
        super::LOG.with_borrow_mut(|log| *log = super::Log::default());
        let line = report
            .lines
            .iter()
            .find(|l| l.name == "effect names")
            .unwrap();
        let text = |s: &str| s.len() + 2 * size_of::<usize>();
        let tables = names * (8 + size_of::<Rc<str>>() + 1) + causes * (8 + size_of::<usize>() + 1);
        assert_eq!(line.bytes, Some(tables + text(&named) + text(&gone)));
    }

    /// A write no maker named says the Rust line that made its signal, as
    /// a frame's log reads it; one a maker named keeps that name.
    #[test]
    fn a_write_no_maker_named_says_the_rust_line_that_made_its_signal() {
        let _tracing = TRACING.lock().unwrap_or_else(|e| e.into_inner());
        super::set_enabled(true);
        let runtime = crate::reactive::Runtime::new();
        let line = line!() + 1;
        let plain = runtime.signal(0);
        let named = runtime.signal(0);
        super::name_signal(named.addr(), "app.luau:3 use_state".into());
        plain.set(1);
        named.set(1);
        let site = |signal: usize| {
            super::LOG.with_borrow(|log| {
                log.current
                    .sites
                    .iter()
                    .find(|(s, _)| *s == signal)
                    .map(|(_, site)| site.to_string())
            })
        };
        let (plain_site, named_site) = (site(plain.addr()), site(named.addr()));
        super::set_enabled(false);
        assert_eq!(plain_site, Some(format!("zgx/src/trace.rs:{line}")));
        assert_eq!(named_site.as_deref(), Some("app.luau:3 use_state"));
    }

    /// An effect no maker named is named by the Rust line that made it; one
    /// its maker named keeps that name.
    #[test]
    fn an_effect_no_maker_named_says_the_rust_line_that_made_it() {
        let _tracing = TRACING.lock().unwrap_or_else(|e| e.into_inner());
        super::set_enabled(true);
        let runtime = crate::reactive::Runtime::new();
        let value = runtime.signal(0);
        let (a, b) = (value.clone(), value.clone());
        let line = line!() + 1;
        let plain = runtime.effect(move || {
            a.get();
        });
        let named = runtime.effect(move || {
            b.get();
        });
        super::name(named.number(), "@12 binding");
        let now_line = line!() + 1;
        let now = runtime.effect_now(|| {});
        value.set(1);
        let name = |effect: &crate::reactive::Effect| {
            super::LOG.with_borrow(|log| log.names.get(&effect.number()).map(|n| n.to_string()))
        };
        let ran = super::LOG.with_borrow(|log| {
            log.current
                .runs
                .iter()
                .rev()
                .find(|run| run.effect == plain.number())
                .and_then(|run| run.name.as_deref().map(str::to_owned))
        });
        let (plain_name, named_name, now_name) = (name(&plain), name(&named), name(&now));
        super::set_enabled(false);
        assert_eq!(plain_name, Some(format!("zgx/src/trace.rs:{line} effect")));
        assert_eq!(ran, plain_name, "its runs say it");
        assert_eq!(named_name.as_deref(), Some("@12 binding"));
        assert_eq!(
            now_name,
            Some(format!("zgx/src/trace.rs:{now_line} effect"))
        );
    }

    /// Held by tests that turn tracing on and off: it is the process's, and
    /// one turning it off would stop another's mid-test.
    pub(crate) static TRACING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sizes() -> (usize, usize) {
        super::LOG.with_borrow(|log| (log.names.len(), log.causes.len()))
    }

    #[test]
    fn remounting_while_tracing_keeps_its_names_bounded() {
        let _tracing = TRACING.lock().unwrap_or_else(|e| e.into_inner());
        super::set_enabled(true);
        let mut ui = Ui::new(200., 200.);
        let count = ui.signal(0u32);
        let shown = ui.signal(true);
        let (read, show) = (count.clone(), shown.clone());
        ui.mount(column().child(switch(
            move || show.get(),
            move |on, _| {
                let read = read.clone();
                column().children((0..if on { 20 } else { 0 }).map(move |i| {
                    let read = read.clone();
                    text_signal(move || format!("{i}: {}", read.get()))
                }))
            },
        )));
        ui.prepare_frame();
        let once = sizes();
        for round in 0..30 {
            // Queued, then taken away before it runs: its cause goes too.
            count.set(round + 1);
            shown.set(false);
            ui.prepare_frame();
            shown.set(true);
            ui.prepare_frame();
        }
        let after = sizes();
        super::set_enabled(false);
        assert!(once.0 > 0, "the mount named its effects");
        assert!(
            after.0 <= once.0 + 4 && after.1 <= once.1 + 4,
            "names and causes stay as many as the mounted effects: {once:?} then {after:?}"
        );
    }
}
