//! A native module for zgx-script's tests (`tests/native_modules.rs`).

use std::{cell::RefCell, rc::Rc};

use zgx_module::{Host, Value};

zgx_module::module!(|host: &Host| {
    // What it was given, back: every kind of value there and back.
    host.function("fixture.echo", |arg| arg);
    // `{ words, longest, kinds }` of a list of strings.
    host.function("fixture.words", |arg| {
        let texts = arg.get();
        let words: usize = texts
            .items()
            .map(|t| t.str().map_or(0, |s| s.split_whitespace().count()))
            .sum();
        let longest = texts
            .items()
            .filter_map(|t| t.str())
            .max_by_key(|s| s.len())
            .unwrap_or("");
        Value::record([
            ("words", Value::from(words as f64)),
            ("longest", Value::from(longest)),
            ("first", texts.item(0).map_or(Value::NIL, Value::from)),
        ])
    });
    // A map's fields as `{ key, value }` pairs, in its order, less `drop`.
    host.function("fixture.fields", |arg| {
        let arg = arg.get();
        let drop = arg.field("drop").and_then(|d| d.str()).unwrap_or("");
        Value::list(
            arg.fields()
                .filter(|(k, _)| *k != drop)
                .map(|(k, v)| Value::record([("key", Value::from(k)), ("value", v.to_owned())])),
        )
    });
    // A value kept by an action, read by a function: a module's own state.
    let kept: Rc<RefCell<Value>> = Rc::default();
    let keep = kept.clone();
    host.action("fixture.keep", move |arg| *keep.borrow_mut() = arg);
    host.function("fixture.kept", move |_| kept.borrow().clone());
});
