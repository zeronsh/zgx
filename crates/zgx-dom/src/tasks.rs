//! zgx's tasks, timers and animation frames in the browser, which has no
//! thread to run them.
//!
//! A component's tasks (`cx.tasks()`, and through them a script's promises,
//! `ui.every`, `ui.after` and `ui.persist`'s saves) run on one executor,
//! drained in a microtask when one of them wakes. zgx's timers are hosted
//! here: one `setTimeout`, armed for the earliest of them and cleared when
//! none waits. The page's [`FrameClock`] ticks from `requestAnimationFrame`,
//! asked for only while an animation awaits a frame (a hidden tab's the
//! browser holds back). An idle page runs nothing.

use crate::js;
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};
use wasm_bindgen::prelude::*;
use web_time::Instant;
use zgx::{
    compose::TaskRunner,
    frame::{Frame, FrameClock},
    task::LocalExecutor,
    timer::Hosted,
};

struct Host {
    executor: Rc<RefCell<LocalExecutor>>,
    timers: Hosted,
    /// The deadline the timeout is armed for.
    armed: Cell<Option<Instant>>,
    frames: FrameClock,
    /// The display's refreshes so far, as frames count them (`Frame::index`).
    refreshes: Cell<u64>,
    /// The last frame's `requestAnimationFrame` time, in ms.
    last_frame: Cell<Option<f64>>,
    /// The display's refresh interval, in ms, as frames have come.
    interval: Cell<f64>,
    _run: Closure<dyn FnMut()>,
    _fire: Closure<dyn FnMut()>,
    _frame: Closure<dyn FnMut(f64)>,
}

thread_local! {
    static HOST: OnceCell<Host> = const { OnceCell::new() };
    /// The executor is running its ready tasks: one woken now (a task
    /// yielding) waits for the browser's next task, so input and paint come
    /// between, and the timeout is armed once they have run.
    static DRAINING: Cell<bool> = const { Cell::new(false) };
}

/// A task woke: run the ready ones soon. Called by the executor's wakers,
/// which only this thread has (the page's wasm has no other).
fn wake() {
    js::zgx_tasks_wake(DRAINING.with(Cell::get));
}

/// The runner and frame clock a mount provides its view. The first call
/// starts the host.
pub(crate) fn services() -> (TaskRunner, FrameClock) {
    HOST.with(|host| {
        let host = host.get_or_init(|| {
            let run = Closure::<dyn FnMut()>::new(run);
            let fire = Closure::<dyn FnMut()>::new(fire);
            let frame = Closure::<dyn FnMut(f64)>::new(frame);
            js::zgx_tasks_init(&run, &fire, &frame);
            Host {
                executor: Rc::new(RefCell::new(LocalExecutor::with_wake(wake))),
                // A sleep that becomes the earliest while tasks run is armed
                // after them; any other arms it in a drain of its own.
                timers: zgx::timer::host(|| {
                    if !DRAINING.with(Cell::get) {
                        js::zgx_tasks_wake(false);
                    }
                }),
                armed: Cell::new(None),
                frames: FrameClock::new(),
                refreshes: Cell::new(0),
                last_frame: Cell::new(None),
                interval: Cell::new(1e3 / 60.),
                _run: run,
                _fire: fire,
                _frame: frame,
            }
        });
        (
            TaskRunner::from_executor(host.executor.clone()),
            host.frames.clone(),
        )
    })
}

/// Run the tasks ready now, then arm the timeout for what they left waiting.
fn run() {
    let polled = HOST.with(|host| {
        let host = host.get()?;
        DRAINING.with(|draining| draining.set(true));
        let polled = host.executor.borrow_mut().tick();
        DRAINING.with(|draining| draining.set(false));
        host.arm();
        host.ask_frame();
        crate::idle::take_requests();
        Some(polled)
    });
    // What they wrote shows before the browser paints.
    if polled.is_some_and(|polled| polled > 0) {
        crate::mount::flush();
    }
}

/// The timeout fired: wake the sleeps due, whose tasks run in a microtask.
fn fire() {
    HOST.with(|host| {
        if let Some(host) = host.get() {
            host.armed.set(None);
            host.timers.fire();
            host.arm();
        }
    });
}

/// A display refresh (`requestAnimationFrame`, at `at` ms of the page's
/// clock): delivered to the animations awaiting one, whose tasks run in a
/// microtask.
fn frame(at: f64) {
    HOST.with(|host| {
        let Some(host) = host.get() else { return };
        let mut interval = host.interval.get();
        let mut refreshes = 1;
        if let Some(last) = host.last_frame.get() {
            let since = at - last;
            if since > 0. && since < interval * 1.5 {
                // A refresh after the last: the interval as it runs, a faster
                // display's taken at once.
                interval = if since < interval * 0.75 {
                    since
                } else {
                    interval * 0.9 + since * 0.1
                };
            } else if since > 0. {
                // Refreshes went by unasked (an idle page, a hidden tab).
                refreshes = (since / interval).round().max(1.) as u64;
            }
        }
        host.interval.set(interval);
        host.last_frame.set(Some(at));
        let index = host.refreshes.get() + refreshes;
        host.refreshes.set(index);
        // Expected on screen a refresh after it began: animations read where
        // they will be seen.
        let late = web_sys::window()
            .and_then(|window| window.performance())
            .map_or(0., |performance| (performance.now() - at).max(0.));
        let interval = std::time::Duration::from_secs_f64(interval / 1e3);
        let time = Instant::now() + interval - std::time::Duration::from_secs_f64(late / 1e3);
        host.frames.deliver(Frame {
            index,
            time,
            interval,
        });
        // One held back by its rate waits for a later refresh.
        host.ask_frame();
    });
}

impl Host {
    /// Ask for the next display refresh if an animation awaits one.
    fn ask_frame(&self) {
        if self.frames.wants_frame() {
            js::zgx_frame_ask();
        }
    }
    /// Arm the timeout for the earliest sleep, or clear it if none waits.
    fn arm(&self) {
        let next = self.timers.deadline();
        if next == self.armed.get() {
            return;
        }
        self.armed.set(next);
        let ms = next.map_or(-1., |at| {
            // `setTimeout` takes up to 2³¹ − 1 ms and fires at once past
            // that: a later sleep is armed again when this fires.
            let ms = at.saturating_duration_since(Instant::now()).as_secs_f64() * 1e3;
            ms.ceil().min(2_147_483_647.)
        });
        js::zgx_timer_arm(ms);
    }
}

/// Whether a timeout is armed for zgx's timers (for tests: an idle page has
/// none).
#[doc(hidden)]
pub fn timer_armed() -> bool {
    js::zgx_timer_armed()
}

/// Whether an animation frame is asked for (for tests: an idle page asks
/// none).
#[doc(hidden)]
pub fn frame_asked() -> bool {
    js::zgx_frame_asked()
}
