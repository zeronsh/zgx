//! Hover follows content scrolling under a still pointer: the row now under
//! it is hovered, the one that scrolled away is not, and the hit test that
//! finds it each scroll frame enters only the row under the pointer: the
//! others are found by search and not looked at, or, where the walk keeps
//! paint order, looked at once and skipped, their content not walked (a
//! scroll's content and its rows are skipped where the pointer isn't, moved
//! as they are).
use zgx::{compose::prelude::*, input::InputEvent, widgets::Ui};

const ROWS: usize = 1000;

/// What is raised (`z`) in the scene, taking its hit tests by the walk that
/// keeps paint order.
#[derive(Clone, Copy, PartialEq)]
enum Raised {
    Nothing,
    /// One of the rows: their column paints out of document order.
    ARow,
    /// A part inside one row (a row's badge, a menu's button): the rows'
    /// column paints in document order.
    InsideARow,
}

#[test]
fn hover_follows_rows_scrolling_under_a_still_pointer_and_its_hit_test_is_bounded() {
    rows_scrolling_under_a_still_pointer(Raised::Nothing);
}

/// The same with one of the rows raised (a `z`), which takes every hit test
/// of the scene by the walk that keeps paint order: bounded, each row looked
/// at once.
#[test]
fn with_a_raised_row_too() {
    rows_scrolling_under_a_still_pointer(Raised::ARow);
}

/// The same with a part of a row raised, as an app's rows raise a badge or
/// a button: the walk keeping paint order still searches the rows, which
/// paint in document order, as the plain one does. (Before, it looked at
/// every row, whatever was raised.)
#[test]
fn with_a_part_of_a_row_raised_the_rows_are_searched() {
    rows_scrolling_under_a_still_pointer(Raised::InsideARow);
}

fn rows_scrolling_under_a_still_pointer(raised: Raised) {
    let mut ui = Ui::new(300., 400.);
    let offset = ui.signal(0_f32);
    let view = ui.mount(
        scroll(offset.clone())
            .w(300.)
            .h(400.)
            .children((0..ROWS).map(|i| {
                // A row of a few parts, as a tool call's row is.
                row()
                    .id(format!("row-{i}"))
                    .w(300.)
                    .h(20.)
                    .hover(|s| s.bg(rgb(0x333333)))
                    .on_click(|| {})
                    .child(div().w(20.).h(20.))
                    .child(
                        column()
                            .child(div().w(200.).h(10.))
                            .child(div().w(200.).h(10.)),
                    )
                    .child(div().w(20.).h(20.))
            })),
    );
    ui.prepare_frame();
    let row = |i: usize| view.find(&format!("row-{i}")).unwrap();
    let lifted = match raised {
        Raised::Nothing => None,
        Raised::ARow => Some(row(500)),
        Raised::InsideARow => Some(ui.scene.borrow().children(row(500))[0]),
    };
    if let Some(lifted) = lifted {
        ui.scene.borrow_mut().set_z(lifted, 1);
        ui.prepare_frame();
    }
    // The pointer rests over the third row and stays there.
    ui.dispatch(InputEvent::PointerMove { x: 50., y: 50. });
    assert_eq!(ui.input.hovered(), Some(row(2)));
    let mut walked = Vec::new();
    for frame in 1..=30 {
        // A wheel's 8 points a frame: the content scrolls under the pointer.
        offset.set(frame as f32 * 8.);
        let before = ui.scene.borrow().hit_nodes_walked();
        ui.prepare_frame();
        walked.push(ui.scene.borrow().hit_nodes_walked() - before);
        // The row under the pointer now: (50 + offset) / 20.
        let under = ((50. + frame as f32 * 8.) / 20.) as usize;
        assert_eq!(ui.input.hovered(), Some(row(under)), "frame {frame}");
    }
    assert_ne!(
        ui.input.hovered(),
        Some(row(2)),
        "the first row scrolled away"
    );
    // Each frame's hit test enters the row under the pointer alone: its
    // parts, not every row's (5 nodes a row; walked whole, 5,000 a frame).
    // It finds that row by searching the rows (`hit_rows`), looking at none
    // of the others, wherever the rows paint in document order; with a row
    // raised, the walk keeping paint order looks at each row once. Debug builds count (a release
    // build says 0).
    if cfg!(debug_assertions) {
        let most = *walked.iter().max().unwrap();
        let bound = if raised == Raised::ARow {
            ROWS as u64 + 16
        } else {
            32
        };
        assert!(most > 0, "a scroll frame refreshes hover");
        assert!(
            most <= bound,
            "a scroll frame's hit test visited {most} nodes: {walked:?}"
        );
    }
}
