//! `text-wrap: balance` and `pretty` break lines as Chrome does (see
//! `zgx::text_wrap`). Headless text is `0.6 em` a character, so 9.6 at 16.
use std::sync::Arc;
use zgx::{
    compose::{IconAlign, SpanIcon, prelude::*},
    scene::{NodeId, NodeKind},
    svg::SvgData,
    widgets::Ui,
};

fn mounted(view: View) -> (Ui, NodeId) {
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(column().w(600.).child(view.id("t")));
    ui.prepare_frame();
    let mut node = handle.find("t").unwrap();
    {
        let scene = ui.scene.borrow();
        while !matches!(
            scene.kind(node),
            NodeKind::Text { .. } | NodeKind::RichText { .. }
        ) {
            node = scene.children(node)[0];
        }
    }
    (ui, node)
}
fn breaks(ui: &Ui, node: NodeId, width: f32) -> Option<Vec<usize>> {
    ui.scene
        .borrow()
        .fitted_breaks(node, width)
        .map(|spaces| spaces.to_vec())
}

// Widths 9.6 a character, lines 134.4 wide: greedy is "aaaa bbbb cccc" / "dd".
// Scores (line 8601, orphan break 10000): S1 9216 + 8601, S2 2304 + 8601,
// S3 10000 + 8601; the last line, balanced, from "bbbb" is 67.2 wide and
// costs 4516 (15421 in all) against 18186 from "aaaa" and 31872 from "cccc".
#[test]
fn balanced_lines_break_as_chrome_balances_them() {
    let (ui, node) = mounted(
        text("aaaa bbbb cccc dd")
            .text_size(16.)
            .w(134.4)
            .text_balance(true),
    );
    assert_eq!(breaks(&ui, node, 134.4), Some(vec![9]));
    let scene = ui.scene.borrow();
    let fitted = scene.fit_str(node, 134.4, &Arc::from("aaaa bbbb cccc dd"));
    let layout = scene.shape_text(&fitted, 16., Some(134.4));
    let (first, second) = (layout.caret(0).y, layout.caret(10).y);
    assert!(second > first, "'cccc' starts the second line");
    assert_eq!(layout.caret(5).y, first, "'bbbb' stays on the first");
}

// Width 96: greedy is "aaaa bbbb" / "ccc dddd" / "ee"; pretty fits the
// short word back: S1 9462, S2 6236, S3 min(9831, 10752) + 6144 = 15975,
// S4 6605 + 10000 + 6144, and the last line from "ccc" is free.
#[test]
fn pretty_fits_a_short_last_line_back() {
    let (ui, node) = mounted(
        text("aaaa bbbb ccc dddd ee")
            .text_size(16.)
            .w(96.)
            .text_pretty(true),
    );
    assert_eq!(breaks(&ui, node, 96.), Some(vec![4, 13]));
}

#[test]
fn text_that_asks_nothing_and_text_that_needs_nothing_breaks_as_before() {
    let (ui, node) = mounted(text("aaaa bbbb ccc dddd ee").text_size(16.).w(96.));
    assert_eq!(breaks(&ui, node, 96.), None);
    let (ui, node) = mounted(
        text("aaaa bbbb cccc dddd")
            .text_size(16.)
            .w(96.)
            .text_pretty(true),
    );
    assert_eq!(breaks(&ui, node, 96.), None, "a last line that isn't short");
    let (ui, node) = mounted(text("Short").text_size(16.).w(300.).text_balance(true));
    assert_eq!(breaks(&ui, node, 300.), None, "one line");
    let long = "aaaaaaaa bbbbbbbb cccccccc dddddddd eeeeeeee ffffffff gggggggg";
    let (ui, node) = mounted(text(long).text_size(16.).w(100.).text_balance(true));
    assert_eq!(breaks(&ui, node, 100.), None, "past six lines");
}

#[test]
fn fitting_is_inherited_and_a_text_can_stop_it() {
    let mut ui = Ui::new(600., 400.);
    let handle = ui.mount(
        column()
            .w(600.)
            .text_balance(true)
            .child(text("aaaa bbbb cccc dd").text_size(16.).w(134.4).id("in"))
            .child(
                text("aaaa bbbb cccc dd")
                    .text_size(16.)
                    .w(134.4)
                    .text_balance(false)
                    .id("out"),
            ),
    );
    ui.prepare_frame();
    let (inner, out) = (handle.find("in").unwrap(), handle.find("out").unwrap());
    assert_eq!(breaks(&ui, inner, 134.4), Some(vec![9]));
    assert_eq!(breaks(&ui, out, 134.4), None);
}

#[test]
fn a_selection_follows_the_fitted_lines() {
    let (ui, node) = mounted(
        text("aaaa bbbb cccc dd")
            .text_size(16.)
            .w(134.4)
            .text_balance(true)
            .selectable(true),
    );
    assert!(ui.select_all_text(node));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let rects = &scene.text_highlight(node).unwrap().rects;
    let right = rects.iter().map(|r| r.x + r.width).fold(0., f32::max);
    assert!(
        right <= 9. * 9.6 + 0.5,
        "{right}: 'bbbb' ends the first line"
    );
}

// Mixed spans with a baseline icon (20 + 4 wide, an inline box) between
// spaces: candidates "aaaa " (48 / 38.4), "icon " (81.6 / 72), "bbbbb "
// (139.2 / 129.6), "cccc " (187.2 / 177.6) and the end (206.4), at width
// 100. Greedy is "aaaa icon" / "bbbbb cccc" / "dd"; scores (line 6400,
// orphan 10000): S1 10195, S2 7184, S3 min(10534, 9888) + 6400 = 16288,
// S4 7200 + 10000 + 6400, and the last line from S3 is free: the lines are
// "aaaa icon " / "bbbbb " / "cccc dd".
#[test]
fn spans_and_baseline_icons_are_measured_where_they_sit() {
    let svg = Arc::new(SvgData::new(&b"<svg viewBox='0 0 24 24'/>"[..]).unwrap());
    let icon = SpanIcon {
        svg,
        size: 20.,
        gap: 4.,
        margin: 0.,
        tint: None,
        bg: None,
        radius: 0.,
        pad: 0.,
        align: IconAlign::Baseline,
    };
    let (ui, node) = mounted(
        rich_text()
            .child(text_span("aaaa "))
            .child(text_span(" bbbbb ").icon(icon))
            .child(text_span("cccc dd"))
            .text_size(16.)
            .w(100.)
            .text_pretty(true)
            .into(),
    );
    // "aaaa " (5), the icon (3 bytes), then " bbbbb " and "cccc dd".
    assert_eq!(breaks(&ui, node, 100.), Some(vec![8, 14]));
}
