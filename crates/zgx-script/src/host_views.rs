//! Views the host makes in Rust, shown where a script puts `ui.host_view { name = "..." }`: in the script's
//! tree, so they lay out, clip and stack like its own elements, under its popups and dialogs.
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use zgx::compose::{View, prelude::*};

type Factory = Rc<dyn Fn() -> View>;

thread_local! {
    static VIEWS: RefCell<HashMap<String, Factory>> = RefCell::default();
}

/// Registers what `ui.host_view { name = name }` shows: `make` runs each time such an element mounts.
pub fn host_view(name: impl Into<String>, make: impl Fn() -> View + 'static) {
    VIEWS.with(|views| views.borrow_mut().insert(name.into(), Rc::new(make)));
}

pub(crate) fn view(name: &str) -> View {
    let make = VIEWS.with(|views| views.borrow().get(name).cloned());
    match make {
        Some(make) => make(),
        None => div(),
    }
}
