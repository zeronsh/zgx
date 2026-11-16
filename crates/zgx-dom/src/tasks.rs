//! zgx's tasks and timers in the browser, which has no thread to run them.
//!
//! A component's tasks (`cx.tasks()`, and through them a script's promises,
//! `ui.every`, `ui.after` and `ui.persist`'s saves) run on one executor,
//! drained in a microtask when one of them wakes. zgx's timers are hosted
//! here: one `setTimeout`, armed for the earliest of them and cleared when
//! none waits, so an idle page runs nothing.

use crate::js;
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};
use wasm_bindgen::prelude::*;
use web_time::Instant;
use zgx::{compose::TaskRunner, task::LocalExecutor, timer::Hosted};

struct Host {
    executor: Rc<RefCell<LocalExecutor>>,
    timers: Hosted,
    /// The deadline the timeout is armed for.
    armed: Cell<Option<Instant>>,
    _run: Closure<dyn FnMut()>,
    _fire: Closure<dyn FnMut()>,
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

/// The runner a mount provides its view. The first call starts the host.
pub(crate) fn runner() -> TaskRunner {
    HOST.with(|host| {
        let host = host.get_or_init(|| {
            let run = Closure::<dyn FnMut()>::new(run);
            let fire = Closure::<dyn FnMut()>::new(fire);
            js::zgx_tasks_init(&run, &fire);
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
                _run: run,
                _fire: fire,
            }
        });
        TaskRunner::from_executor(host.executor.clone())
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

impl Host {
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
