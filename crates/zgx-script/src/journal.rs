//! What a load wrote to state that outlives it (`ui.state`, `ui.persist`,
//! `ui.shared`), so a load that fails undoes it (`ScriptHost::retarget`,
//! `reload`): each value's first write, with what it was, and each key the
//! load made. Recorded only while a load runs after one that succeeded
//! (`active`): a host's first load has nothing to go back to, so it records
//! nothing, and any other write asks one flag.
use crate::{Value, persist::Persist, value::Store};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use zgx::reactive::Signal;

/// A key a load made, removed if it fails.
enum Made {
    State(Rc<Store>, String),
    Persist(Rc<Persist>, Rc<str>, String),
}

#[derive(Default)]
pub(crate) struct Journal {
    active: Cell<bool>,
    /// A load succeeded: those after it record.
    armed: Cell<bool>,
    /// Each value written, once, with what it held before.
    written: RefCell<Vec<(Signal<Value>, Value)>>,
    made: RefCell<Vec<Made>>,
}

impl Journal {
    /// Whether a load is running: what a write asks before anything else.
    #[inline]
    pub(crate) fn active(&self) -> bool {
        self.active.get()
    }
    /// A load begins: recorded if one succeeded before it.
    pub(crate) fn begin(&self) {
        self.written.borrow_mut().clear();
        self.made.borrow_mut().clear();
        self.active.set(self.armed.get());
    }
    /// The load succeeded: what it wrote stays.
    pub(crate) fn keep(&self) {
        self.active.set(false);
        self.armed.set(true);
        *self.written.borrow_mut() = Vec::new();
        *self.made.borrow_mut() = Vec::new();
    }
    /// The load failed: each value back to what it held, last written
    /// first, and each key it made gone.
    pub(crate) fn undo(&self) {
        self.active.set(false);
        let written = std::mem::take(&mut *self.written.borrow_mut());
        let made = std::mem::take(&mut *self.made.borrow_mut());
        for (signal, was) in written.into_iter().rev() {
            signal.set(was);
        }
        for made in made {
            match made {
                Made::State(store, key) => store.remove(&key),
                Made::Persist(persist, script, key) => persist.remove(&script, &key),
            }
        }
    }
    /// `signal` about to be written: what it holds kept, if it's the
    /// first write to it this load.
    #[cold]
    #[inline(never)]
    pub(crate) fn write(&self, signal: &Signal<Value>) {
        let mut written = self.written.borrow_mut();
        if written.iter().any(|(kept, _)| kept.ptr_eq(signal)) {
            return;
        }
        written.push((signal.clone(), signal.with_untracked(Clone::clone)));
    }
    /// `key` made in `store` (`ui.state`, `ui.shared`).
    #[cold]
    #[inline(never)]
    pub(crate) fn made_state(&self, store: &Rc<Store>, key: &str) {
        self.made
            .borrow_mut()
            .push(Made::State(store.clone(), key.to_owned()));
    }
    /// `script`'s `key` made in `persist` (`ui.persist`).
    #[cold]
    #[inline(never)]
    pub(crate) fn made_persist(&self, persist: &Rc<Persist>, script: &Rc<str>, key: &str) {
        self.made.borrow_mut().push(Made::Persist(
            persist.clone(),
            script.clone(),
            key.to_owned(),
        ));
    }
}
