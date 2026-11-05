//! `ui.list { data = s, key = "id" }`: rows follow their items as with a
//! function, and keying a long list makes none of its items a table: only the
//! rows being built reach the script.
mod common;
use common::Harness;
use std::rc::Rc;
use zgx_script::Value;

const APP: &str = r#"
local rows = ui.shared("rows", {})
return ui.list {
    id = "list", w = 400, h = 300, height = 20, measured = true,
    data = rows,
    key = "id",
    row = function(row)
        local marked = ui.use_state(false)
        return ui.button {
            id = row.id, h = 20, on_click = function() marked:set(true) end,
            function() return if marked:get() then "marked" else "plain" end,
        }
    end,
}
"#;

fn rows(ids: impl IntoIterator<Item = usize>) -> Value {
    let rows: Vec<Value> = ids
        .into_iter()
        .map(|i| {
            let parts: Vec<Value> = (0..8)
                .map(|p| Value::record([("n", Value::Num(p as f64))]))
                .collect();
            Value::record([
                ("id", Value::from(format!("row-{i}").as_str())),
                ("parts", Value::List(Rc::from(parts))),
            ])
        })
        .collect();
    Value::List(Rc::from(rows))
}

#[test]
fn a_field_key_makes_only_the_rows_being_built_tables() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    let before = zgx_script::values_converted();
    h.host.shared("rows", Value::Nil).set(rows(0..5000));
    h.frames(3);
    let converted = zgx_script::values_converted() - before;
    assert_eq!(h.error(), None);
    assert!(h.find("#row-0").is_some(), "the first rows are shown");
    // 5,000 rows of 8 parts are some 90,000 items; 15 rows in view and its
    // overscan, some 300.
    assert!(converted < 2000, "converted {converted} items");
    // An edit converts no more than that row.
    let before = zgx_script::values_converted();
    h.host.shared("rows", Value::Nil).set(rows(0..5001));
    h.frames(3);
    assert!(zgx_script::values_converted() - before < 2000);
}

/// What the test above is measured against: the same list keyed by a function.
#[test]
fn a_function_key_makes_every_new_item_a_table() {
    let h = Harness::inline(&APP.replace(r#"key = "id""#, "key = function(row) return row.id end"));
    h.frames(2);
    let before = zgx_script::values_converted();
    h.host.shared("rows", Value::Nil).set(rows(0..5000));
    h.frames(3);
    let converted = zgx_script::values_converted() - before;
    assert_eq!(h.error(), None);
    assert!(converted > 40_000, "converted {converted} items");
}

#[test]
fn a_field_key_keeps_a_rows_state_as_the_list_moves() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    h.host.shared("rows", Value::Nil).set(rows([1, 2, 3]));
    h.frames(3);
    h.click("row-2");
    h.frames(2);
    assert_eq!(h.text("row-2"), "marked");
    h.host.shared("rows", Value::Nil).set(rows([9, 1, 2, 3]));
    h.frames(3);
    assert_eq!(h.text("row-2"), "marked", "moved, not built again");
    assert_eq!(h.text("row-9"), "plain");
}

#[test]
fn a_row_without_the_field_keys_by_its_place() {
    let h = Harness::inline(APP);
    h.frames(2);
    let plain = Value::List(Rc::from(vec![Value::record([("other", Value::Num(1.))])]));
    h.host.shared("rows", Value::Nil).set(plain);
    h.frames(3);
    assert_eq!(h.error(), None, "as with a function giving nil");
}

#[test]
fn a_field_key_wants_data() {
    let code = r#"return ui.list { w = 100, h = 100, height = 20, count = 3, key = "id",
        row = function(i) return ui.text { h = 20, "x" } end }"#;
    let failed = zgx_script::testing::TestApp::try_new(|runtime| {
        zgx_script::ScriptHost::inline(runtime, "list", code)
    });
    let error = failed.err().unwrap_or_default();
    assert!(error.contains("takes `data`"), "{error}");
}
