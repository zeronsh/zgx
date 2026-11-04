//! A long list made into a table for a script is the host's work, not the
//! script's: it costs a call none of its budget.
mod common;
use common::Harness;
use std::rc::Rc;
use std::time::Duration;
use zgx_script::{Limits, ScriptHost, Value};

/// A computed that does almost nothing but read the list, and then reaches
/// the checks a spent budget stops at.
const READ: &str = r#"
local rows = ui.shared("rows", {})
local total = ui.computed(function(): number
    local all = rows:get()
    local n = 0
    for i = 1, 3000 do
        n = n + i % 2
    end
    return #all * #(if all[1] then all[1].parts else {})
end)
return ui.text { id = "total", function() return tostring(total:get()) end }
"#;

/// A list asked for its items' keys, which is most of what it does.
const KEYS: &str = r#"
local rows = ui.shared("rows", {})
return ui.list {
    id = "list", w = 400, h = 300, height = 20, measured = true,
    data = rows,
    key = function(row) return row.id end,
    row = function(row) return ui.text { id = row.id, h = 20, row.id } end,
}
"#;

/// 300 rows, each large: so much to convert, so little for a script to do.
fn long_rows() -> Value {
    let parts: Vec<Value> = (0..1000)
        .map(|i| {
            Value::record([
                ("kind", Value::from("text")),
                ("text", Value::from(format!("part {i}").as_str())),
            ])
        })
        .collect();
    let parts = Value::List(Rc::from(parts));
    let rows: Vec<Value> = (0..300)
        .map(|i| {
            // (Each its own: a table shared between rows converts once.)
            let parts = Value::List(Rc::from(
                parts
                    .list()
                    .iter()
                    .map(|p| match p {
                        Value::Map(_) => Value::record([
                            ("text", p.field("text").cloned().unwrap_or(Value::Nil)),
                            ("row", Value::Num(i as f64)),
                        ]),
                        other => other.clone(),
                    })
                    .collect::<Vec<_>>(),
            ));
            Value::record([
                ("id", Value::from(format!("row-{i}").as_str())),
                ("parts", parts),
            ])
        })
        .collect();
    Value::List(Rc::from(rows))
}

/// Far below what 300,000 records take to convert in any build, and far above
/// what these scripts do: a few reads, and 300 keys.
fn tight() -> Limits {
    Limits {
        call: Duration::from_millis(10),
        load: Duration::from_secs(10),
        ..Default::default()
    }
}

#[test]
fn reading_a_long_list_costs_a_call_none_of_its_budget() {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "list", READ).limits(tight()));
    h.frames(2);
    h.host.shared("rows", Value::Nil).set(long_rows());
    h.frames(3);
    assert_eq!(h.error(), None, "no budget spent by the conversion");
    assert_eq!(
        h.text("total"),
        (300 * 1000).to_string(),
        "the computed ran to its end"
    );
}

#[test]
fn keying_a_long_list_costs_a_call_none_of_its_budget() {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "list", KEYS).limits(tight()));
    h.frames(2);
    h.host.shared("rows", Value::Nil).set(long_rows());
    h.frames(3);
    assert_eq!(h.error(), None, "no budget spent by the conversion");
    assert!(h.find("#row-0").is_some(), "keyed by the items' ids");
}
