//! A row dragged out of a `ui.list` is one node from the press to the drop,
//! as a sidebar's row is: its place folding away while it's held (to
//! nothing, or a sliver) and rows opening above it don't make it again, so
//! its focus, hover and accessible element go on with it.
mod common;

use common::Harness;
use zgx::input::{InputEvent, PointerButton};

/// Rows a to e, e folded away; dragging one folds its place to `folded` as
/// the drag starts (in its `on_drag`, or with `later` a beat after, as a
/// sidebar's does), and, with `zone`, opens a row above them all while it's
/// held, as a sidebar's drop zone does.
fn list(folded: &str, zone: bool, later: bool) -> String {
    format!(
        r##"local held = ui.shared("held", "")
        local keys = {{ "a", "b", "c", "d", "e" }}
        local rows = ui.computed(function()
            local out = {{}}
            if {zone} and held:get() ~= "" then
                table.insert(out, "zone")
            end
            for _, k in keys do
                table.insert(out, k)
            end
            return out
        end)
        local heights = ui.computed(function()
            local out = {{}}
            for i, k in rows:get() do
                out[i] = if k == held:get() or k == "e" then {folded} else 30
            end
            return out
        end)
        return ui.list {{ id = "list", w = 200, h = 400, heights = heights,
            key = function(i) return rows:get()[i] end,
            row = function(key)
                return ui.div {{ id = key, w_full = true, h = 30,
                    on_drag = function()
                        if {later} then
                            ui.after(0, function() held:set(key) end)
                        else
                            held:set(key)
                        end
                        return {{ row = key }}
                    end,
                    on_drag_end = function()
                        held:set("")
                    end,
                }}
            end,
        }}"##
    )
}

/// Drags `c` well away and lets go: its node as pressed, held and dropped,
/// and folded-away `e`'s while it was held.
fn drag_c(h: &mut Harness) -> [Option<zgx::scene::NodeId>; 4] {
    h.settle();
    h.frames(2);
    let pressed = h.find("c");
    let b = h.bounds("c");
    let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
    let button = PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x, y });
    h.ui.dispatch(InputEvent::PointerDown { x, y, button });
    for step in 1..=6 {
        let y = y + 20. * step as f32;
        h.ui.dispatch(InputEvent::PointerMove { x, y });
        h.frames(2);
    }
    h.settle();
    h.frames(2);
    assert!(h.ui.input.drag_state().is_some(), "a drag under way");
    let held = h.find("c");
    let e = h.find("e");
    h.ui.dispatch(InputEvent::PointerUp {
        x,
        y: y + 120.,
        button,
    });
    h.settle();
    h.frames(2);
    [pressed, held, h.find("c"), e]
}

#[test]
fn a_dragged_row_whose_place_folds_is_one_node_through_the_drop() {
    for folded in ["0", "0.001"] {
        for later in [true, false] {
            let mut h = Harness::inline(&list(folded, false, later));
            assert_eq!(h.error(), None);
            let [pressed, held, dropped, _] = drag_c(&mut h);
            assert!(pressed.is_some());
            assert_eq!(
                held, pressed,
                "{folded}, later {later}: held, the row it was pressed"
            );
            assert_eq!(
                dropped, pressed,
                "{folded}, later {later}: dropped, the same row"
            );
        }
    }
}

#[test]
fn a_dragged_row_stays_one_node_as_rows_open_above_it() {
    for later in [true, false] {
        let mut h = Harness::inline(&list("0.001", true, later));
        assert_eq!(h.error(), None);
        let [pressed, held, dropped, _] = drag_c(&mut h);
        assert!(pressed.is_some());
        assert_eq!(held, pressed, "later {later}: held, the row it was pressed");
        assert_eq!(dropped, pressed, "later {later}: dropped, the same row");
    }
}

#[test]
fn rows_folded_away_are_still_not_built_during_a_drag() {
    // Only the row the drag started from is kept: the rest folded away
    // aren't built, as before.
    let mut h = Harness::inline(&list("0.001", true, true));
    let [.., e] = drag_c(&mut h);
    assert_eq!(e, None, "e, folded away");
}
