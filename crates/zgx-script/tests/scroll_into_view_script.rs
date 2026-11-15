//! `el:scroll_into_view(options)` from a script: a row of a scroll view
//! brought into it where asked, whether anything moved, and a wrong
//! alignment named; `ui.scroll_into_view(handle)` the same for a control
//! `ui.controls` listed.
mod common;

use common::Harness;

const APP: &str = r##"
local target = ui.ref()
local offset = ui.state("offset", 0)
local moved = ui.state("moved", "none")
local function reveal(options)
    moved:set(tostring(target:peek():scroll_into_view(options)))
end
ui.command { id = "start", run = function() reveal { block = "start", animate = false } end }
ui.command { id = "nearest", run = function() reveal { block = "nearest", animate = false } end }
ui.command { id = "center", run = function() reveal { block = "center", animate = false } end }
ui.command { id = "wrong", run = function() reveal { block = "middle" } end }
-- By a control's handle, as `ui.press` takes one: r1's button, its handle
-- kept while it shows.
local first = -1
ui.command { id = "scan", run = function()
    for _, c in ui.controls() do
        if c.name == "First" then
            first = c.handle
        end
    end
end }
ui.command { id = "down", run = function() offset:set(300) end }
ui.command { id = "by_handle", run = function()
    moved:set(tostring(ui.scroll_into_view(first, { block = "start", animate = false })))
end }
ui.command { id = "stale", run = function() moved:set(tostring(ui.scroll_into_view(1e15))) end }
local rows = {}
for i = 1, 10 do
    rows[i] = ui.div { id = `r{i}`, h = 40, ref = if i == 6 then target else nil,
        if i == 1 then ui.button { "First", h = 40, on_click = function() end } else nil }
end
return ui.column {
    ui.text { id = "moved", "{moved}" },
    ui.text { id = "offset", function() return tostring(offset:get()) end },
    ui.scroll { id = "list", offset = offset, w = 200, h = 100, table.unpack(rows) },
}
"##;

#[test]
fn a_script_scrolls_a_row_into_view_where_it_asks() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    // r6 is 200 to 240 down the rows.
    assert!(h.host.run_command("start"));
    h.frames(2);
    assert_eq!(h.text("moved"), "true");
    assert_eq!(h.text("offset"), "200");
    assert!(h.host.run_command("nearest"));
    h.frames(2);
    assert_eq!(h.text("moved"), "false", "it shows: nothing moves");
    assert!(h.host.run_command("center"));
    h.frames(2);
    assert_eq!(
        (h.text("moved"), h.text("offset")),
        ("true".into(), "170".into())
    );
}

#[test]
fn ui_scrolls_a_controls_handle_into_view() {
    let mut h = Harness::inline(APP);
    h.frames(2);
    assert!(h.host.run_command("scan"));
    assert!(h.host.run_command("down"));
    h.frames(2);
    assert_eq!(h.text("offset"), "300");
    // The button fills r1, at the top of the rows: back to it.
    assert!(h.host.run_command("by_handle"));
    h.frames(2);
    assert_eq!(
        (h.text("moved"), h.text("offset")),
        ("true".into(), "0".into())
    );
    assert!(h.host.run_command("stale"));
    h.frames(2);
    assert_eq!(
        h.text("moved"),
        "false",
        "a handle to nothing moves nothing"
    );
}

#[test]
fn a_wrong_alignment_is_named() {
    let h = Harness::inline(APP);
    h.frames(2);
    h.host.run_command("wrong");
    h.frames(1);
    let error = h.error().unwrap_or_default();
    assert!(
        error.contains("scroll_into_view's block") && error.contains("middle"),
        "{error:?}"
    );
}
