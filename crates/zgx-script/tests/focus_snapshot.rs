//! `ui.focus_snapshot()` and `:restore()`: what had the keyboard, and a
//! field's selection, put back after a guide took the user elsewhere,
//! nothing scrolled.
mod common;
use common::Harness;

const APP: &str = r##"
local log = ui.state("log", "")
local shown = ui.state("shown", true)
local text = ui.state("text", "hello wide world")
local els = {}
local snap = nil
local function keep(name: string)
    return function(el) els[name] = el end
end
-- Focus `name` (where it is), select `a..b` and take a snapshot; then put
-- the selection elsewhere, for the restore to bring back.
local function take(name: string, a: number, b: number)
    return function()
        local el = els[name]
        el:focus({ scroll = false })
        el:set_selection(a, b)
        snap = ui.focus_snapshot()
        el:set_selection(0)
        log:set(if snap then "taken" else "nil")
    end
end
return ui.column {
    w_full = true,
    -- Not focusable: its press leaves nothing focused.
    ui.div { id = "empty", w = 10, h = 10, focusable = false, on_click = function()
        log:set(tostring(ui.focus_snapshot()))
    end },
    ui.button { id = "take_field", w = 10, h = 10, on_click = take("field", 12, 6) },
    ui.button { id = "take_area", w = 10, h = 10, on_click = take("area", 3, 9) },
    ui.button { id = "take_code", w = 10, h = 10, on_click = take("code", 6, 11) },
    ui.button { id = "take_far", w = 10, h = 10, on_click = take("far", 1, 2) },
    ui.button { id = "restore", w = 10, h = 10, on_click = function()
        log:set(`{snap:restore()}`)
    end },
    ui.button { id = "hide", w = 10, h = 10, on_click = function() shown:set(false) end },
    ui.button { id = "shorten", w = 10, h = 10, on_click = function() text:set("hi") end },
    ui.text { id = "log", "{log}" },
    ui.input { id = "field", label = "Field", value = text, w = 200, h = 24, on_mount = keep("field") },
    ui.textarea { id = "area", label = "Area", value = ui.state("a", "one\ntwo\nthree"), w = 200, h = 60,
        on_mount = keep("area") },
    ui.show(shown, ui.textarea { id = "code", label = "Code", value = ui.state("c", "local x = 1"),
        w = 200, h = 60, on_mount = keep("code"),
        highlight = function(_line: string, state: any) return {}, state end }),
    ui.scroll { id = "scroller", w = 200, h = 60,
        ui.div { h = 400 },
        ui.input { id = "far", label = "Far", value = ui.state("f", "away"), w = 200, h = 24,
            on_mount = keep("far") },
    },
}
"##;

fn harness() -> Harness {
    let h = Harness::inline(APP);
    h.ui.scene.borrow_mut().resize(400., 600.);
    h.frames(2);
    h
}

#[test]
fn nothing_focused_is_nil() {
    let mut h = harness();
    assert_eq!(h.ui.input.focused(), None);
    h.click("empty");
    assert_eq!(h.text("log"), "nil");
}

#[test]
fn each_editor_kind_round_trips_its_selection() {
    // Characters from 0, as `set_selection` counts them; bytes for these
    // ASCII texts too.
    for (take, id, anchor, focus) in [
        ("take_field", "field", 12, 6),
        ("take_area", "area", 3, 9),
        ("take_code", "code", 6, 11),
    ] {
        let mut h = harness();
        h.click(take);
        assert_eq!(h.text("log"), "taken", "{id}");
        let node = h.find(id).unwrap();
        let editor = h.ui.editor(node).unwrap();
        assert_eq!(editor.selection(), (0, 0), "{id}: moved before the restore");
        h.click("restore");
        assert_eq!(h.text("log"), "true", "{id}");
        assert_eq!(h.ui.input.focused(), Some(node), "{id}");
        assert_eq!(editor.selection(), (anchor, focus), "{id}");
    }
}

#[test]
fn an_unmounted_field_is_false() {
    let mut h = harness();
    h.click("take_code");
    h.click("hide");
    h.frames(1);
    assert!(h.find("code").is_none());
    h.click("restore");
    assert_eq!(h.text("log"), "false");
}

#[test]
fn offsets_past_the_end_are_false() {
    let mut h = harness();
    h.click("take_field");
    h.click("shorten");
    h.frames(1);
    h.click("restore");
    assert_eq!(h.text("log"), "false");
    let field = h.find("field").unwrap();
    assert_ne!(h.ui.input.focused(), Some(field));
}

#[test]
fn restoring_scrolls_nothing() {
    let mut h = harness();
    let before = h.bounds("far");
    h.click("take_far");
    h.frames(1);
    assert_eq!(h.bounds("far"), before, "taken where it is");
    h.click("restore");
    h.frames(2);
    assert_eq!(h.text("log"), "true");
    assert_eq!(h.ui.input.focused(), h.find("far"));
    assert_eq!(
        h.bounds("far"),
        before,
        "the scroll view stays where it was"
    );
}
