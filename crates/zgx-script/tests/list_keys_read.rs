//! A list's keys read the short fields of its items, not the items: a
//! measured list needs a key for every row before it shows any, and rows off
//! the screen stay out of the script's heap, however long their text.
mod common;

use common::Harness;
use std::rc::Rc;
use zgx_script::Value;

const TEXT: usize = 20_000;

fn rows(n: usize) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            Value::record(vec![
                ("id", Value::from(format!("row-{i}").as_str())),
                ("meta", Value::record(vec![("id", Value::from(i as f64))])),
                // Each its own: the script's heap keeps equal strings once.
                (
                    "text",
                    Value::from(format!("{i} {}", "x".repeat(TEXT)).as_str()),
                ),
            ])
        })
        .collect();
    Value::List(rows.into())
}

fn app(key: &str) -> String {
    format!(
        r##"local rows = ui.shared("rows", {{}})
return ui.column {{
    height = 200,
    ui.list {{
        id = "list",
        grow = 1,
        data = rows,
        measured = true,
        key = function(m)
            return {key}
        end,
        row = function(m)
            return ui.text {{ id = "row-" .. m.id, height = 20, m.id }}
        end,
    }},
}}"##
    )
}

fn shown(key: &str) -> (Harness, usize) {
    let h = Harness::inline(&app(key));
    let shared = h.host.shared("rows", Value::List(Rc::from([])));
    h.settle();
    h.host.collect_garbage();
    let before = h.host.script_heap();
    shared.set(rows(2_000));
    h.settle();
    h.host.collect_garbage();
    (h, before)
}

#[test]
fn rows_off_the_screen_stay_out_of_the_script_for_their_keys() {
    let (mut h, before) = shown("m.id");
    let grew = h.host.script_heap().saturating_sub(before);
    // 40 MB of text in the list; a screenful of rows is 200 KB of it.
    assert!(grew < 3_000_000, "the script's heap grew {grew} bytes");
    assert!(h.text("row-row-0").contains("row-0"), "the first row shows");
}

#[test]
fn a_key_reading_more_than_the_short_fields_is_given_the_item() {
    // Nested, long, absent or looped over: each reads what the item has.
    for (key, first) in [
        ("m.meta.id", "row-0"),
        ("#m.text + m.meta.id", "row-0"),
        ("m.absent or m.id", "row-0"),
        (
            "(function() local n = 0; for _ in m do n += 1 end; assert(n == 3); return m.id end)()",
            "row-0",
        ),
    ] {
        let (mut h, _) = shown(key);
        assert_eq!(h.host.error().get(), None, "{key}");
        assert!(h.text(&format!("row-{first}")).contains(first), "{key}");
    }
}

#[test]
fn a_keyed_row_follows_its_item_as_the_list_is_reversed() {
    let mut h = Harness::inline(&app("m.id"));
    let shared = h.host.shared("rows", Value::List(Rc::from([])));
    h.settle();
    shared.set(rows(40));
    h.settle();
    assert!(h.text("row-row-0").contains("row-0"));
    let Value::List(items) = rows(40) else {
        unreachable!()
    };
    let reversed: Vec<Value> = items.iter().rev().cloned().collect();
    shared.set(Value::List(reversed.into()));
    h.settle();
    assert!(h.text("row-row-39").contains("row-39"), "the last is first");
}
