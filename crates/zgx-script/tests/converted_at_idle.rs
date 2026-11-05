//! What a script was shown and has let go of doesn't stay: a long list read
//! once, then replaced, is let go of as the window goes idle, in Rust and in
//! the script's heap, however large the cache had grown.
mod common;

use common::Harness;
use std::rc::Rc;
use zgx_script::Value;

fn rows(n: usize) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            Value::record(vec![
                ("id", Value::from(format!("row-{i}").as_str())),
                ("text", Value::from("x".repeat(400).as_str())),
            ])
        })
        .collect();
    Value::List(rows.into())
}

const APP: &str = r##"local rows = ui.shared("rows", {})
return ui.column {
    ui.text { id = "count", function() return tostring(#rows:get()) end },
}"##;

#[test]
fn a_long_list_replaced_is_let_go_of_as_the_window_goes_idle() {
    let mut h = Harness::inline(APP);
    let shared = h.host.shared("rows", Value::List(Rc::from([])));
    h.settle();
    h.host.collect_garbage();
    let before = h.host.script_heap();
    let long = rows(5_000);
    let Value::List(items) = &long else {
        unreachable!()
    };
    let held = Rc::downgrade(items);
    shared.set(long.clone());
    h.settle();
    assert_eq!(h.text("count"), "5000", "the script read it");
    assert!(h.host.script_heap() > before + 1_000_000, "converted");
    drop(long);
    // A short list next: nothing that doubles the cache.
    shared.set(rows(3));
    h.settle();
    assert_eq!(h.text("count"), "3");
    h.host.collect_garbage();
    assert!(held.upgrade().is_none(), "nothing keeps the long list");
    let after = h.host.script_heap();
    assert!(
        after < before + 200_000,
        "the script's heap comes back: {before} then {after}"
    );
}

/// A value the script still has is its own, and stays the same table.
#[test]
fn what_a_state_still_shows_stays_converted() {
    let mut h = Harness::inline(APP);
    let shared = h.host.shared("rows", Value::List(Rc::from([])));
    let long = rows(1_000);
    shared.set(long.clone());
    h.settle();
    h.host.collect_garbage();
    assert_eq!(h.text("count"), "1000");
    let calls = h.calls();
    h.frames(3);
    assert_eq!(h.calls(), calls, "nothing to convert again");
    assert_eq!(h.error(), None);
    drop(long);
}
