//! The page's idle trim, as a desktop window's (`zgx::idle`): once its DOM
//! has been still for `TRIM_DELAY`, what was registered with
//! `zgx::idle::on_idle` runs for each mounted view's runtime. A script host
//! collects its VMs' garbage there: Luau's collector runs only as scripts
//! allocate, so without it what a script let go of (rows cleared, a view
//! gone) stayed held until the script next ran a while, and raised the next
//! peak, which wasm memory keeps for good.
//!
//! A flush that writes the DOM counts itself, nothing more: no clock read and
//! no timer call per frame. One timeout is armed at a time. When it fires
//! having seen writes since it was armed, it waits again; once a whole delay
//! has passed with none, the trim runs, 1.5 to 3 s after the last write. Then
//! nothing is armed, so a page at rest runs nothing.

use crate::js;
use std::cell::{Cell, OnceCell, RefCell};
use wasm_bindgen::prelude::*;
use zgx::reactive::{Runtime, WeakRuntime};

/// How long the DOM is still before the trim runs: the desktop's.
const TRIM_DELAY_MS: f64 = 1500.;

#[derive(Default)]
struct Trim {
    /// The mounted views' runtimes.
    runtimes: RefCell<Vec<WeakRuntime>>,
    /// Flushes that wrote the DOM, ever.
    writes: Cell<u64>,
    /// `writes` as the timeout was armed.
    seen: Cell<u64>,
    armed: Cell<bool>,
    /// Running: what it frees writes the DOM, and asks for no trim again.
    trimming: Cell<bool>,
    fire: OnceCell<Closure<dyn FnMut()>>,
}

thread_local! {
    static TRIM: Trim = Trim::default();
}

/// Marks a runtime the trim knows (`Runtime::extension`): each is listed
/// once however many views of it mount.
struct Watched(Cell<bool>);

/// A view of `runtime` mounted: trimmed as the page goes idle.
pub(crate) fn watch(runtime: &Runtime) {
    let watched = runtime.extension(|_| Watched(Cell::new(false)));
    if watched.0.replace(true) {
        return;
    }
    TRIM.with(|trim| {
        let mut runtimes = trim.runtimes.borrow_mut();
        runtimes.retain(|weak| weak.upgrade().is_some());
        runtimes.push(runtime.downgrade());
    });
}

/// A flush wrote the DOM (or a trim was asked for): the page isn't idle.
#[inline]
pub(crate) fn wrote() {
    TRIM.with(|trim| {
        if trim.trimming.get() {
            return;
        }
        trim.writes.set(trim.writes.get() + 1);
        if !trim.armed.get() {
            arm(trim);
        }
    });
}

/// Whether a runtime asked for a trim (`zgx::idle::request`): then one
/// comes, as for a write.
pub(crate) fn take_requests() {
    let asked = TRIM.with(|trim| {
        trim.runtimes
            .borrow()
            .iter()
            .filter_map(WeakRuntime::upgrade)
            .fold(false, |asked, runtime| {
                zgx::idle::take_request(&runtime) | asked
            })
    });
    if asked {
        wrote();
    }
}

fn arm(trim: &Trim) {
    trim.armed.set(true);
    trim.seen.set(trim.writes.get());
    let fire = trim.fire.get_or_init(|| Closure::<dyn FnMut()>::new(fire));
    js::zgx_trim_arm(fire, TRIM_DELAY_MS);
}

/// The timeout fired: written since it was armed, it waits again;
/// otherwise the trim runs.
fn fire() {
    let runtimes = TRIM.with(|trim| {
        trim.armed.set(false);
        if trim.writes.get() != trim.seen.get() {
            arm(trim);
            return None;
        }
        trim.trimming.set(true);
        let mut runtimes = trim.runtimes.borrow_mut();
        runtimes.retain(|weak| weak.upgrade().is_some());
        Some(
            runtimes
                .iter()
                .filter_map(WeakRuntime::upgrade)
                .collect::<Vec<_>>(),
        )
    });
    let Some(runtimes) = runtimes else { return };
    for runtime in &runtimes {
        zgx::idle::trim(runtime);
    }
    TRIMS.with(|trims| trims.set(trims.get() + 1));
    // What it freed (nodes a collection let go of) leaves the DOM now.
    crate::mount::flush();
    TRIM.with(|trim| trim.trimming.set(false));
}

/// Whether the idle trim's timeout is armed (for tests: a page at rest has
/// none once it has run).
#[doc(hidden)]
pub fn trim_armed() -> bool {
    TRIM.with(|trim| trim.armed.get())
}

/// How many times the idle trim has run (for tests).
#[doc(hidden)]
pub fn trims() -> u64 {
    TRIMS.with(Cell::get)
}

thread_local! {
    static TRIMS: Cell<u64> = const { Cell::new(0) };
}
