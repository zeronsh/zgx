//! An anchored panel is 240 wide unless the script says otherwise; `w = "auto"`
//! (and `"max-content"`) over that default sizes it to what it holds.
mod common;
use common::Harness;

/// The panel's width, and the width it would be if it held its child and its
/// own padding alone: the child's width, plus where the child starts inside it
/// on each side. (The padding is the platform's theme's: 16 on macOS, 8 on
/// Linux.)
fn panel_width(width: &str, child: u32) -> (f32, f32) {
    let mut h = Harness::inline(&format!(
        r##"local open = ui.state("open", true)
        return ui.div {{ w_full = true, h_full = true,
            ui.popover {{ id = "panel", open = open, {width}
                anchor = ui.button {{ "Open" }},
                build = function() return ui.div {{ id = "child", w = {child}, h = 20 }} end }} }}"##
    ));
    h.frames(3);
    assert_eq!(h.error(), None);
    let (panel, held) = (h.bounds("panel"), h.bounds("child"));
    assert_eq!(held.width, child as f32, "the child keeps its width");
    let padding = held.x - panel.x;
    assert!(padding > 0., "the panel is padded: {padding}");
    (panel.width, held.width + 2. * padding)
}

#[test]
fn a_panel_is_240_wide_by_default() {
    assert_eq!(panel_width("", 100).0, 240.);
    assert_eq!(panel_width("", 300).0, 240.);
}

#[test]
fn auto_sizes_a_panel_to_what_it_holds() {
    for child in [100, 300] {
        let (width, held) = panel_width(r#"w = "auto","#, child);
        assert_eq!(width, held, "a {child}-wide child");
    }
}

#[test]
fn max_content_sizes_a_panel_to_what_it_holds() {
    for child in [100, 300] {
        let (width, held) = panel_width(r#"w = "max-content","#, child);
        assert_eq!(width, held, "a {child}-wide child");
    }
}

#[test]
fn a_width_over_auto_still_wins() {
    assert_eq!(panel_width(r#"w = 200,"#, 100).0, 200.);
}
