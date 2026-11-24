//! Where the Rust callback a script called was registered, told only as a
//! panic unwinds out of it: a host's action, function or fetcher panicking
//! deep in a library names the line in the host's code that handed it to
//! zgx (`ScriptHost::action` and its kin keep it, `#[track_caller]`), for
//! a contained panic's report.
//!
//! A call through [`run`] costs nothing more on its way back: the note is
//! made by a guard's `Drop`, which runs only while unwinding (a normal
//! return forgets it). The innermost callback's site is kept.
use std::{cell::Cell, panic::Location};

/// A callback's kind ("a host action") and where it was registered.
pub type Site = (&'static str, &'static Location<'static>);

thread_local! {
    static SITE: Cell<Option<Site>> = const { Cell::new(None) };
}

/// Notes `site` if a panic unwinds through it, and nothing else.
struct Unwinding(Site);
impl Drop for Unwinding {
    #[cold]
    fn drop(&mut self) {
        // The innermost callback's: the first to unwind.
        let _ = SITE.try_with(|slot| {
            if slot.get().is_none() {
                slot.set(Some(self.0));
            }
        });
    }
}

/// Run `f`, a callback of `kind` registered at `at`: if it panics, the
/// unwind notes where it was registered ([`take`]).
#[inline(always)]
pub fn run<R>(kind: &'static str, at: &'static Location<'static>, f: impl FnOnce() -> R) -> R {
    let unwinding = Unwinding((kind, at));
    let returned = f();
    std::mem::forget(unwinding);
    returned
}

/// The site the last panic unwound through, taken (a panic's reporter
/// reads it once).
pub fn take() -> Option<Site> {
    SITE.try_with(Cell::take).ok().flatten()
}

/// Forget any site left by an earlier panic: a panic hook calls it as a
/// panic begins, before it unwinds through anything.
pub fn clear() {
    let _ = SITE.try_with(|slot| slot.set(None));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_unwinding_through_a_callback_notes_its_site_and_a_return_does_not() {
        clear();
        let at = Location::caller();
        assert_eq!(run("a host action", at, || 1), 1);
        assert!(take().is_none(), "a normal return notes nothing");
        let inner = Location::caller();
        let caught = std::panic::catch_unwind(|| {
            run("a host action", at, || {
                run("the fetcher", inner, || panic!("deep"))
            })
        });
        assert!(caught.is_err());
        let (kind, site) = take().expect("noted as it unwound");
        assert_eq!(kind, "the fetcher", "the innermost callback's");
        assert_eq!(site.line(), inner.line());
        assert!(take().is_none(), "taken once");
    }
}
