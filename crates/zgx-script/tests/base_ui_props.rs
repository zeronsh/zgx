//! Luau props that follow Base UI: `hover_delay` and `placement` collision options on popovers,
//! `tooltip_close_delay` and `tooltip_group`, `initial_focus` and `final_focus`, and a span icon's
//! `align`.
mod common;
use common::Harness;
use zgx::input::InputEvent;
use zgx_script::{ScriptHost, testing::TestApp};

fn try_inline(code: &str) -> Result<TestApp, String> {
    let code = code.to_owned();
    TestApp::try_new(move |runtime| ScriptHost::inline(runtime, "app", &code))
}
fn focused(h: &Harness) -> Option<zgx::scene::NodeId> {
    h.ui.input.focused()
}
fn focus(h: &mut Harness, id: &str) {
    let node = h.find(id);
    h.ui.input.focus(&h.ui.scene, node);
}

fn move_to(h: &mut Harness, x: f32, y: f32) {
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.frames(2);
}
fn center(h: &mut Harness, id: &str) -> (f32, f32) {
    let node = h.find(id).unwrap();
    let b = h.ui.scene.borrow().bounds(node);
    (b.x + b.width / 2., b.y + b.height / 2.)
}

const HOVER: &str = r##"
local open = ui.state("open", false)
return ui.column { p = 40,
    ui.text { id = "state", function() return tostring(open:get()) end },
    ui.popover { open = open, hover_delay = { open = 0, close = 0 },
        anchor = ui.button { id = "trigger", "Hover" },
        ui.text { "card" } },
}
"##;

#[test]
fn hover_delay_opens_a_popover_as_the_pointer_rests_and_closes_it_as_it_leaves() {
    let mut h = Harness::inline(HOVER);
    h.frames(3);
    let (x, y) = center(&mut h, "trigger");
    move_to(&mut h, x, y);
    assert_eq!(h.text("state"), "true", "opens on the trigger");
    move_to(&mut h, 700., 500.);
    assert_eq!(h.text("state"), "false", "closes off it");
}

#[test]
fn a_hover_delay_that_is_no_table_is_an_error() {
    let result = try_inline(
        r#"return ui.popover { open = ui.state("o", false), hover_delay = 5, anchor = ui.button { "x" }, ui.text { "c" } }"#,
    );
    assert!(result.is_err(), "hover_delay takes a table");
}

#[test]
fn a_hover_delay_on_a_menu_and_a_submenu_loads() {
    let mut h = Harness::inline(
        r##"return ui.column {
            ui.menu { open = ui.state("m", false), hover_delay = { open = 200, close = 100 },
                anchor = ui.button { "Menu" },
                ui.submenu { open = ui.state("s", false), hover_delay = { open = 50 },
                    ui.menu_item { "Child" } } },
        }"##,
    );
    h.frames(2);
    assert_eq!(h.error(), None);
}

/// A popover whose trigger is at the window's right edge, its panel's right edge, the window's width.
fn panel_right(placement: &str) -> (f32, f32) {
    let mut h = Harness::inline(&format!(
        r##"local open = ui.state("open", true)
        return ui.row {{ w_full = true, justify = "end",
            ui.popover {{ open = open, placement = {placement},
                anchor = ui.button {{ id = "trigger", w = 60, "T" }},
                ui.div {{ id = "card", w = 200, h = 40 }} }},
        }}"##
    ));
    h.frames(4);
    let card = h.find("card").unwrap();
    let b = h.ui.scene.borrow().bounds(card);
    let window = h.ui.scene.borrow().bounds(h.ui.root()).width;
    (b.x + b.width, window)
}

#[test]
fn placement_padding_keeps_a_popover_that_far_off_the_window_edge() {
    let (plain, width) = panel_right("{ side = \"bottom\" }");
    assert!(
        plain <= width + 0.5,
        "kept in the window: {plain} of {width}"
    );
    let (padded, _) = panel_right("{ side = \"bottom\", padding = 30 }");
    assert!(
        padded <= width - 30. + 0.5,
        "30 off the edge: {padded} of {width}"
    );
    let (loose, _) = panel_right("{ side = \"bottom\", shift = false }");
    assert!(
        loose > width,
        "no shift: it keeps its aligned place and reaches past: {loose} of {width}"
    );
}

#[test]
fn a_flip_align_collision_option_loads_and_places() {
    let (right, width) = panel_right("{ side = \"bottom\", flip_align = true, more_room = true }");
    assert!(right <= width + 0.5, "{right} of {width}");
}

#[test]
fn tooltip_close_delay_and_group_props_load() {
    let mut h = Harness::inline(
        r##"return ui.column { p = 40,
            tooltip_group = { delay = 100, close_delay = 200, reshow_within = 500, reshow_delay = 0 },
            ui.button { id = "a", "A", tooltip = "Tip A", tooltip_close_delay = 300 },
            ui.button { id = "b", "B", tooltip = "Tip B" },
        }"##,
    );
    h.frames(3);
    assert_eq!(h.error(), None);
    assert!(h.find("a").is_some() && h.find("b").is_some());
}

const FOCUS: &str = r##"
local open = ui.state("open", false)
ui.command { id = "open", run = function() open:set(true) end }
ui.command { id = "close", run = function() open:set(false) end }
return ui.column { gap = 8,
    ui.button { id = "trigger", "Open", on_click = function() open:set(true) end },
    ui.button { id = "other", "Other" },
    ui.modal { open = open, w = 240, p = 16,
        initial_focus = "second", final_focus = "other",
        ui.button { id = "first", "First" },
        ui.button { id = "second", "Second" } },
}
"##;

#[test]
fn initial_and_final_focus_name_where_focus_goes_in_a_dialog() {
    let mut h = Harness::inline(FOCUS);
    h.frames(3);
    focus(&mut h, "trigger");
    assert!(h.host.run_command("open"));
    h.frames(3);
    assert_eq!(focused(&h), h.find("second"), "initial_focus = \"second\"");
    assert!(h.host.run_command("close"));
    h.frames(3);
    assert_eq!(focused(&h), h.find("other"), "final_focus = \"other\"");
}

#[test]
fn initial_focus_false_leaves_no_control_focused() {
    let mut h = Harness::inline(
        r##"local open = ui.state("open", false)
        ui.command { id = "open", run = function() open:set(true) end }
        return ui.column {
            ui.modal { open = open, initial_focus = false, final_focus = false,
                ui.button { id = "first", "First" } },
        }"##,
    );
    h.frames(3);
    assert!(h.host.run_command("open"));
    h.frames(3);
    assert_ne!(focused(&h), h.find("first"));
}

#[test]
fn a_focus_prop_that_is_neither_an_id_nor_false_is_an_error() {
    let result = try_inline(
        r#"return ui.modal { open = ui.state("o", true), initial_focus = 5, ui.button { "x" } }"#,
    );
    assert!(result.is_err());
}

#[test]
fn a_span_icon_on_the_baseline_loads_and_grows_the_line() {
    let line = |align: &str| {
        let mut h = Harness::inline(&format!(
            r##"return ui.column {{ ui.rich {{ id = "t", wrap = true, text_size = 14, line_height = 20, w = 300,
                ui.span {{ "word", icon = {{ code = "<svg viewBox='0 0 24 24'/>", size = 56, gap = 4, {align} }} }} }} }}"##
        ));
        h.frames(4);
        assert_eq!(h.error(), None);
        let node = h.find("t").unwrap();
        let height = h.ui.scene.borrow().bounds(node).height;
        height
    };
    let centred = line("align = \"center\"");
    let baseline = line("align = \"baseline\"");
    assert!(centred < 30., "centred, the line keeps its 20: {centred}");
    assert!(
        baseline > 56.,
        "on the baseline the line grows to hold 56: {baseline}"
    );
}

#[test]
fn an_icon_align_that_is_no_known_one_is_an_error() {
    let result = try_inline(
        r##"return ui.rich { ui.span { "x", icon = { code = "<svg/>", align = "top" } } }"##,
    );
    assert!(result.is_err());
}
