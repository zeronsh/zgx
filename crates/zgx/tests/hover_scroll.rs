//! Hover follows content scrolling under a still pointer: the row now under
//! it is hovered, the one that scrolled away is not, and the hit test that
//! finds it each scroll frame enters only the row under the pointer: the
//! others are found by search and not looked at, or, where the walk keeps
//! paint order, looked at once and skipped, their content not walked (a
//! scroll's content and its rows are skipped where the pointer isn't, moved
//! as they are).
use zgx::{compose::prelude::*, input::InputEvent, widgets::Ui};

const ROWS: usize = 1000;

#[test]
fn hover_follows_rows_scrolling_under_a_still_pointer_and_its_hit_test_is_bounded() {
    rows_scrolling_under_a_still_pointer(false);
}

/// The same with a raised node elsewhere in the window (a sticky header, a
/// `z`), which takes every hit test of the scene by the walk that keeps
/// paint order: bounded as the plain one is.
#[test]
fn with_a_raised_node_in_the_scene_too() {
    rows_scrolling_under_a_still_pointer(true);
}

fn rows_scrolling_under_a_still_pointer(raised: bool) {
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
    if raised {
        let raised = row(500);
        ui.scene.borrow_mut().set_z(raised, 1);
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
    // In a plain scene it finds that row by searching the rows (`hit_rows`),
    // looking at none of the others; with a raised node, whose walk keeps
    // paint order, it looks at each row once. Debug builds count (a release
    // build says 0).
    if cfg!(debug_assertions) {
        let most = *walked.iter().max().unwrap();
        let bound = if raised { ROWS as u64 + 16 } else { 32 };
        assert!(most > 0, "a scroll frame refreshes hover");
        assert!(
            most <= bound,
            "a scroll frame's hit test visited {most} nodes: {walked:?}"
        );
    }
}
