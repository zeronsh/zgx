//! A pointer sweeping across rows with a hover style repaints only when the
//! row under it changes, not on every move within one.
use zgx::input::InputEvent;
mod common;
use common::Harness;

/// Moves sweeping rows 28 tall at 600 points a second (5 points a move, 120 a
/// second) down the test window (300 tall); the moves after which a frame paints.
const STEPS: usize = 58;
fn painting_moves(row: &str) -> (usize, usize) {
    let mut h = Harness::inline(&format!(
        r##"return ui.column {{ w = 300, h = 640,
            ui.list {{ id = "rows", w = 300, h = 640, height = 28, count = 200,
                row = function(i) return {row} end }},
        }}"##
    ));
    h.ui.prepare_frame();
    h.ui.scene.borrow_mut().flush();
    let (mut moves, mut painted) = (0, 0);
    for step in 0..STEPS {
        let y = 4. + step as f32 * 5.;
        h.ui.dispatch(InputEvent::PointerMove { x: 100., y });
        h.ui.prepare_frame();
        let report = h.ui.scene.borrow_mut().flush();
        moves += 1;
        if !report.damage.is_empty() {
            painted += 1;
        }
    }
    (moves, painted)
}

#[test]
fn hover_rows_paint_when_the_row_under_the_pointer_changes() {
    let plain = painting_moves(r##"ui.row { h = 28, ui.text { "Row " .. i } }"##);
    let hover = painting_moves(
        r##"ui.row { h = 28, hover = { bg = "#334455" }, ui.text { "Row " .. i } }"##,
    );
    let button = painting_moves(
        r##"ui.button { h = 28, variant = "ghost", on_click = function() end, ui.text { "Row " .. i } }"##,
    );
    // The rows the sweep crosses into: what a hover style changes.
    let row = |step: usize| ((4. + step as f32 * 5.) / 28.) as usize;
    let changes = (1..STEPS)
        .filter(|&step| row(step) != row(step - 1))
        .count()
        + 1;
    assert_eq!(plain, (STEPS, 0), "no hover style: nothing to paint");
    assert_eq!(hover, (STEPS, changes), "a hover style: each new row, once");
    assert_eq!(button, (STEPS, changes), "a ghost button's hover: the same");
}
