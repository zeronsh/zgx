//! A menu's panel is 240 wide unless the script says otherwise; `w = "auto"` sizes it to its widest row
//! (as the web's does), its other rows stretched to that width.
mod common;
use common::Harness;

fn menu(width: &str, rows: &str) -> Harness {
    let h = Harness::inline(&format!(
        r##"local open = ui.state("open", true)
        return ui.div {{ w_full = true, h_full = true,
            ui.menu {{ id = "panel", open = open, {width}
                anchor = ui.button {{ "Open" }},
                build = function() return ui.column {{ {rows} }} end }} }}"##
    ));
    h.frames(3);
    assert_eq!(h.error(), None);
    h
}
const WIDE: &str = r#"ui.div { id = "a", w = 100, h = 20 }, ui.div { id = "b", w = 300, h = 20 },"#;

#[test]
fn a_menu_is_240_wide_by_default() {
    assert_eq!(menu("", WIDE).bounds("panel").width, 240.);
}

#[test]
fn auto_sizes_a_menu_to_its_widest_row() {
    // The panel's 16-point padding each side.
    assert_eq!(menu(r#"w = "auto","#, WIDE).bounds("panel").width, 332.);
    assert_eq!(
        menu(r#"w = "auto","#, r#"ui.div { id = "a", w = 100, h = 20 },"#)
            .bounds("panel")
            .width,
        132.
    );
}

#[test]
fn a_menus_rows_fill_the_panel_whatever_its_width() {
    let rows = r#"ui.menu_item { id = "short", "Copy" }, ui.menu_item { id = "long", "Look Up this very long selection and then some" },"#;
    let mut h = menu("", rows);
    assert_eq!(h.bounds("panel").width, 240.);
    assert_eq!(h.bounds("short").width, 208., "a row fills the 240 panel");
    let mut h = menu(r#"w = "auto","#, rows);
    let (short, long) = (h.bounds("short").width, h.bounds("long").width);
    assert_eq!(short, long, "rows share the widest's width");
    assert_eq!(long, h.bounds("panel").width - 32.);
}

fn context_menu_width(style: &str, label: &str) -> f32 {
    use zgx::{
        input::{InputEvent, PointerButton},
        semantics::Role,
    };
    let mut h = Harness::inline(&format!(
        r##"return ui.column {{
            ui.div {{ id = "target", w = 200, h = 100,
                {style}
                context_menu = function(e)
                    return {{ ui.menu_item {{ "{label}", id = "item" }} }}
                end }},
        }}"##
    ));
    h.ui.prepare_frame();
    let target = h.find("target").unwrap();
    let b = h.ui.scene.borrow().bounds(target);
    let (x, y) = (b.x + 25., b.y + 10.);
    for event in [
        InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Secondary,
        },
    ] {
        h.ui.dispatch(event);
    }
    h.settle();
    h.ui.prepare_frame();
    assert_eq!(h.error(), None);
    let item = h.find("item").expect("the menu shows");
    let scene = h.ui.scene.borrow();
    let semantics = h.ui.semantics.borrow();
    let panel = scene
        .ancestors(item)
        .find(|node| semantics.get(*node).is_some_and(|s| s.role == Role::Menu))
        .expect("a menu panel");
    scene.bounds(panel).width
}

#[test]
fn a_context_menu_with_an_auto_width_sizes_to_its_widest_row_between_its_min_and_max() {
    const LONG: &str = "Look Up this selection and the rest of its long sentence here";
    let auto = r#"context_menu_style = { w = "auto", min_w = 160, max_w = 320 },"#;
    assert_eq!(context_menu_width("", "Copy"), 240., "by default");
    assert_eq!(
        context_menu_width(auto, "Copy"),
        160.,
        "a short row: its min"
    );
    assert_eq!(context_menu_width(auto, LONG), 320., "a long one: its max");
    // Between them, a row's own width: neither the default nor a bound.
    let fits = context_menu_width(auto, "Look Up this selection");
    assert!(fits > 160. && fits < 320. && fits != 240., "{fits}");
}
