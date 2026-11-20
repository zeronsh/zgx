//! An anchored panel is 240 wide unless the script says otherwise; `w = "auto"`
//! (and `"max-content"`) over that default sizes it to what it holds.
mod common;
use common::Harness;

fn panel_width(width: &str, child: u32) -> f32 {
    let mut h = Harness::inline(&format!(
        r##"local open = ui.state("open", true)
        return ui.div {{ w_full = true, h_full = true,
            ui.popover {{ id = "panel", open = open, {width}
                anchor = ui.button {{ "Open" }},
                build = function() return ui.div {{ w = {child}, h = 20 }} end }} }}"##
    ));
    h.frames(3);
    assert_eq!(h.error(), None);
    h.bounds("panel").width
}

#[test]
fn a_panel_is_240_wide_by_default() {
    assert_eq!(panel_width("", 100), 240.);
    assert_eq!(panel_width("", 300), 240.);
}

#[test]
fn auto_sizes_a_panel_to_what_it_holds() {
    // The panel's 16-point padding each side.
    assert_eq!(panel_width(r#"w = "auto","#, 100), 132.);
    assert_eq!(panel_width(r#"w = "auto","#, 300), 332.);
}

#[test]
fn max_content_sizes_a_panel_to_what_it_holds() {
    assert_eq!(panel_width(r#"w = "max-content","#, 100), 132.);
    assert_eq!(panel_width(r#"w = "max-content","#, 300), 332.);
}

#[test]
fn a_width_over_auto_still_wins() {
    assert_eq!(panel_width(r#"w = 200,"#, 100), 200.);
}
