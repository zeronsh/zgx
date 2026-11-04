//! A failed `key` leaves the rows it already knew on their keys, so their
//! nodes and states stay.
mod common;
use common::Harness;
use std::rc::Rc;
use zgx_script::Value;

fn long_rows(n: usize) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| Value::record([("id", Value::from(format!("row-{i}").as_str()))]))
        .collect();
    Value::List(Rc::from(rows))
}

const FAILS: &str = r#"
local rows = ui.shared("rows", {})
local broken = ui.state("broken", false)
ui.command { id = "break", run = function() broken:set(true) end }
ui.command { id = "mend", run = function() broken:set(false) end }
return ui.list {
    id = "list", w = 400, h = 300, height = 20, count = function() return #rows:get() end,
    measured = true,
    key = function(i)
        if broken:get() then
            error("no key today")
        end
        return rows:at(i).id
    end,
    row = function(key)
        local marked = ui.use_state(false)
        return ui.button {
            id = key, h = 20, on_click = function() marked:set(true) end,
            function() return if marked:get() then "marked" else "plain" end,
        }
    end,
}
"#;

#[test]
fn a_failed_key_keeps_the_keys_rows_had_so_they_keep_their_state() {
    let mut h = Harness::inline(FAILS);
    h.frames(2);
    h.host.shared("rows", Value::Nil).set(long_rows(3));
    h.frames(3);
    h.click("row-2");
    h.frames(2);
    assert_eq!(h.text("row-2"), "marked");
    assert!(h.host.run_command("break"));
    h.frames(3);
    assert!(h.error().is_some(), "the failure is still reported");
    assert_eq!(
        h.text("row-2"),
        "marked",
        "the row wasn't made again by index"
    );
    assert!(h.host.run_command("mend"));
    h.frames(3);
    assert_eq!(h.text("row-2"), "marked", "nor when its key came back");
}
