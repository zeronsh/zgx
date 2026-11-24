//! A measured list resting at its top keeps its top when a row comes in
//! above (a feed's newest shows); scrolled down, it keeps the row in view
//! where it is (CSS's scroll anchoring).
use zgx::{
    compose::{ViewHandle, prelude::*},
    reactive::Signal,
    widgets::Ui,
};

/// Rows 60 tall, estimated at 40, in a 400-point list; the row keyed 0 is
/// `head` tall.
fn feed(ui: &mut Ui, keys: Signal<Vec<u32>>, head: f32) -> (Signal<f32>, ViewHandle) {
    let offset = ui.signal(0_f32);
    let heights = VariableHeights::new(&ui.runtime, 0, 40.);
    let rows = keys.clone();
    let view = ui.mount(
        measured_rows(
            offset.clone(),
            heights,
            1,
            move || rows.get(),
            |_: &[u32], _| 40.,
            move |_, key, _| {
                div()
                    .id(format!("row-{key}"))
                    .h(if key == 0 { head } else { 60. })
            },
        )
        .size(600., 400.),
    );
    (offset, view)
}
fn settle(ui: &mut Ui) {
    for _ in 0..10 {
        ui.prepare_frame();
    }
}
/// Row `key`'s top in the window, if it shows.
fn top(ui: &Ui, view: &ViewHandle, key: u32) -> Option<f32> {
    let scene = ui.scene.borrow();
    view.find(&format!("row-{key}"))
        .and_then(|id| scene.contains(id).then(|| scene.transformed_bounds(id).y))
}

#[test]
fn a_row_added_above_a_list_at_its_top_shows_at_the_top() {
    for head in [40., 56., 100.] {
        let mut ui = Ui::new(800., 600.);
        let keys = ui.signal((1..=30_u32).collect::<Vec<_>>());
        let (offset, view) = feed(&mut ui, keys.clone(), head);
        settle(&mut ui);
        let list_top = top(&ui, &view, 1).expect("row 1 shows");
        assert_eq!(offset.get(), 0., "resting at the top");
        keys.update(|keys| keys.insert(0, 0));
        settle(&mut ui);
        assert_eq!(offset.get(), 0., "a head {head} tall: still at the top");
        assert_eq!(
            top(&ui, &view, 0),
            Some(list_top),
            "the head {head} tall at the top"
        );
        assert_eq!(top(&ui, &view, 1), Some(list_top + head), "row 1 below it");
    }
}

#[test]
fn a_row_added_above_a_scrolled_list_leaves_the_row_in_view_where_it_is() {
    for head in [40., 56., 100.] {
        let mut ui = Ui::new(800., 600.);
        let keys = ui.signal((1..=30_u32).collect::<Vec<_>>());
        let (offset, view) = feed(&mut ui, keys.clone(), head);
        settle(&mut ui);
        // Row 6 at the top of the view, 10 points of it above.
        offset.set(5. * 60. + 10.);
        settle(&mut ui);
        let before = top(&ui, &view, 6).expect("row 6 shows");
        keys.update(|keys| keys.insert(0, 0));
        settle(&mut ui);
        assert_eq!(
            top(&ui, &view, 6),
            Some(before),
            "row 6 where it was (head {head})"
        );
        // The head, above the view, is not built: its estimate holds its place.
        assert_eq!(
            offset.get(),
            5. * 60. + 10. + 40.,
            "moved down by the head's estimate"
        );
    }
}

#[test]
fn a_list_kept_at_its_end_stays_there_as_a_row_comes_in_at_the_top() {
    let mut ui = Ui::new(800., 600.);
    let keys = ui.signal((1..=30_u32).collect::<Vec<_>>());
    let offset = ui.signal(f32::MAX);
    let heights = VariableHeights::new(&ui.runtime, 0, 40.).anchor_end();
    let rows = keys.clone();
    let view = ui.mount(
        measured_rows(
            offset.clone(),
            heights,
            1,
            move || rows.get(),
            |_: &[u32], _| 40.,
            |_, key, _| {
                div()
                    .id(format!("row-{key}"))
                    .h(if key == 0 { 56. } else { 60. })
            },
        )
        .size(600., 400.),
    );
    settle(&mut ui);
    let list_bottom = top(&ui, &view, 30).expect("the last row shows") + 60.;
    keys.update(|keys| keys.insert(0, 0));
    settle(&mut ui);
    assert_eq!(
        top(&ui, &view, 30).map(|t| t + 60.),
        Some(list_bottom),
        "still at its end"
    );
}
