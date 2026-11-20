//! A native module for zgx-script's tests (`tests/native_modules.rs`).

use std::{cell::RefCell, rc::Rc};

use zgx_module::{Host, Value};

zgx_module::module!(|host: &Host| {
    // What it was given, back: every kind of value there and back.
    host.function("fixture.echo", |arg| arg.to_owned());
    // `{ words, longest, kinds }` of a list of strings.
    host.function("fixture.words", |texts| {
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
        let drop = arg.record().get("drop").and_then(|d| d.str()).unwrap_or("");
        Value::list(
            arg.fields()
                .filter(|(k, _)| *k != drop)
                .map(|(k, v)| Value::record([("key", Value::from(k)), ("value", v.to_owned())])),
        )
    });
    // A list's numbers summed (a pass over more items than one read holds),
    // and its items `from` to `to` as a list.
    host.function("fixture.sum", |list| {
        Value::from(list.items().filter_map(|n| n.num()).sum::<f64>())
    });
    host.function("fixture.slice", |arg| {
        let arg = arg.record();
        let at = |key| arg.get(key).and_then(|n| n.num()).unwrap_or(0.) as usize;
        arg.get("list")
            .map_or(Value::NIL, |list| list.slice(at("from"), at("to")))
    });
    // How many fields a map has, and its last key: a map wider than a
    // record holds in place.
    host.function("fixture.wide", |map| {
        let record = map.record();
        let last = record.iter().last().map_or("", |(k, _)| k);
        Value::record([
            ("n", Value::from(record.len() as f64)),
            ("last", Value::from(last)),
        ])
    });
    // A value kept by an action, read by a function: a module's own state.
    let kept: Rc<RefCell<Value>> = Rc::default();
    let keep = kept.clone();
    host.action("fixture.keep", move |arg| {
        *keep.borrow_mut() = arg.to_owned()
    });
    host.function("fixture.kept", move |_| kept.borrow().clone());
});
