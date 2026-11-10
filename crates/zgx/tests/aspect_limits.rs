//! A minimum or a maximum through an aspect ratio, as CSS transfers one: to an
//! axis whose own size is unset only. A definite width keeps its width, and
//! the ratio's height is then held to the height's own limits.
use std::sync::Arc;
use zgx::{compose::prelude::*, image::ImageData, scene::Rect, widgets::Ui};

fn first(view: View) -> Rect {
    let mut ui = Ui::new(800., 600.);
    let handle = ui.mount(view);
    ui.prepare_frame();
    ui.scene.borrow().bounds(handle.find("a").unwrap())
}

#[test]
fn a_definite_width_keeps_its_width_under_a_maximum_height() {
    // CSS: `width: 100%; aspect-ratio: 0.75; max-height: 320px` in a 400 px
    // column is 400x320, where carrying the maximum across made it 240x320.
    let pixels = Arc::new(ImageData::new(1200, 1600, vec![255; 1200 * 1600 * 4]).unwrap());
    let photo = image("a", pixels)
        .id("a")
        .w_full()
        .aspect_ratio(0.75)
        .max_h(320.);
    let rect = first(column().w(400.).child(photo).child(div().id("b")));
    assert_eq!((rect.width, rect.height), (400., 320.));
    // A box as an image.
    let panel = div().id("a").w_full().aspect_ratio(0.75).max_h(320.);
    let rect = first(column().w(400.).child(panel).child(div().id("b")));
    assert_eq!((rect.width, rect.height), (400., 320.));
}

#[test]
fn a_definite_width_keeps_its_width_over_a_minimum_height() {
    // CSS: `width: 400px; aspect-ratio: 0.75; min-height: 600px` is 400x600.
    let panel = div().id("a").w(400.).aspect_ratio(0.75).min_h(600.);
    let rect = first(column().items_start().child(panel).child(div().id("b")));
    assert_eq!((rect.width, rect.height), (400., 600.));
}

#[test]
fn a_limit_still_passes_to_a_side_left_unset() {
    // CSS: `height: 100px; aspect-ratio: 2; max-height: 50px`, its width
    // unset: the maximum passes to the width as 100, so 100x50.
    let panel = div().id("a").h(100.).aspect_ratio(2.).max_h(50.);
    let rect = first(
        row()
            .w(400.)
            .items_start()
            .child(panel)
            .child(div().id("b")),
    );
    assert_eq!((rect.width, rect.height), (100., 50.));
}
