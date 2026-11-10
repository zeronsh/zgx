//! An image given an `aspect_ratio`, its width decided by flex: as CSS's
//! `<img style="aspect-ratio: …">`, the ratio sets its height from that width,
//! whatever its own pixels' shape.
use std::sync::Arc;
use zgx::{compose::prelude::*, image::ImageData, layout::Length, scene::Rect, widgets::Ui};

/// A 1200x1600 picture's pixels.
fn portrait() -> Arc<ImageData> {
    Arc::new(ImageData::new(1200, 1600, vec![255; 1200 * 1600 * 4]).unwrap())
}

fn rects(view: View) -> Vec<Rect> {
    let mut ui = Ui::new(800., 600.);
    let handle = ui.mount(view);
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    ["a", "b"]
        .iter()
        .map(|id| scene.bounds(handle.find(id).unwrap()))
        .collect()
}

fn flexed(item: View) -> View {
    item.grow()
        .flex_basis(Length::Px(0.))
        .min_w(0.)
        .aspect_ratio(1.)
}
fn photo(id: &str) -> View {
    flexed(image(id, portrait()).id(id))
}
fn panel(id: &str) -> View {
    flexed(div().id(id))
}

#[test]
fn flexed_images_with_an_aspect_ratio_are_its_shape_not_their_pixels() {
    // CSS: two `flex: 1 1 0; min-width: 0; aspect-ratio: 1` images of a
    // 1200x1600 photo in a 400 px row are 200x200 squares (198 with a gap),
    // aligned or stretched.
    for (row, side) in [
        (row().w(400.).items_start(), 200.),
        (row().w(400.), 200.),
        (row().w(400.).gap(4.).items_start(), 198.),
    ] {
        let rects = rects(row.child(photo("a")).child(photo("b")));
        for rect in &rects {
            assert_eq!((rect.width, rect.height), (side, side), "{rects:?}");
        }
        assert_eq!(rects[1].x, 400. - side);
    }
    // In a column, as a chat row's pictures are.
    let rects = rects(
        column().w(400.).child(
            row()
                .w_full()
                .gap(4.)
                .items_start()
                .child(photo("a"))
                .child(photo("b")),
        ),
    );
    assert_eq!((rects[0].width, rects[0].height), (198., 198.), "{rects:?}");
}

#[test]
fn flexed_boxes_with_an_aspect_ratio_are_its_shape() {
    // An empty box as an image: CSS gives it the ratio's height, not none.
    let rects = rects(
        row()
            .w(400.)
            .items_start()
            .child(panel("a"))
            .child(panel("b")),
    );
    for rect in &rects {
        assert_eq!((rect.width, rect.height), (200., 200.), "{rects:?}");
    }
}

#[test]
fn an_image_with_an_aspect_ratio_and_one_size_takes_the_other_from_it() {
    // CSS: `width: 300px; aspect-ratio: 2` is 300x150, `height: 100px;
    // aspect-ratio: 2` 200x100, and an aspect ratio alone keeps the
    // picture's own width (1200) and sets the height from it.
    for (item, size) in [
        (
            image("a", portrait()).id("a").w(300.).aspect_ratio(2.),
            (300., 150.),
        ),
        (
            image("a", portrait()).id("a").h(100.).aspect_ratio(2.),
            (200., 100.),
        ),
        (
            image("a", portrait()).id("a").aspect_ratio(2.),
            (1200., 600.),
        ),
    ] {
        let rects = rects(
            row()
                .w(2000.)
                .items_start()
                .child(item)
                .child(div().id("b")),
        );
        assert_eq!((rects[0].width, rects[0].height), size, "{rects:?}");
    }
}

#[test]
fn an_image_without_an_aspect_ratio_keeps_its_pixels_shape() {
    // Not this: an image sized by its width alone scales as it always has.
    let rects = rects(
        row()
            .w(400.)
            .items_start()
            .child(image("a", portrait()).id("a").w(150.))
            .child(div().id("b")),
    );
    assert_eq!(rects[0].width, 150.);
    let before = rects[0].height;
    assert!(before > 150., "{rects:?}");
}
