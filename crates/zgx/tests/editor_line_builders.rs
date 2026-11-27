//! A line's room, its marks and a scroll group's frame are built outside
//! zgx, as an app builds them: `new` (or the default) and a builder for each
//! field, named as the field is (the structs gain fields, so a literal of
//! one compiles only in zgx). A field without its builder fails here.
use std::sync::Arc;
use zgx::{
    decoration::Decoration,
    image::ImageData,
    scene::Color,
    style::ObjectFit,
    svg::SvgData,
    text_layout::TextAlign,
    widgets::{GroupFrame, LineInset, LineMark, MarkImage, MarkLook},
};

const INK: Color = Color(1, 2, 3, 255);

#[test]
fn every_field_of_a_mark_has_a_builder() {
    let image = Arc::new(ImageData::new(1, 1, vec![9_u8; 4]).unwrap());
    let svg = Arc::new(
        SvgData::new(br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"##.to_vec())
            .unwrap(),
    );
    let decoration = Arc::new(Decoration::default());
    let mark = LineMark::default()
        .x(1.)
        .y(2.)
        .width(3.)
        .height(4.)
        .bottom(5.)
        .fill(INK)
        .radius(6.)
        .border_color(INK)
        .border_width(7.)
        .from_right(true)
        .mono(true)
        .label((Arc::from("1."), INK, 8.))
        .image(MarkImage::cover(image.clone()))
        .align(TextAlign::Center)
        .svg(svg.clone())
        .decoration(decoration.clone())
        .pinned(true)
        .over_text(true)
        .rotate(90.)
        .tabular(true);
    assert_eq!(
        (mark.x, mark.y, mark.width, mark.height, mark.bottom),
        (1., 2., 3., 4., 5.)
    );
    assert_eq!((mark.fill, mark.radius), (INK, 6.));
    assert_eq!((mark.border_color, mark.border_width), (INK, 7.));
    assert!(mark.from_right && mark.pinned && mark.over_text);
    assert!(mark.is_mono() && mark.is_tabular());
    let plain = mark.clone().mono(false);
    assert!(!plain.is_mono() && plain.is_tabular());
    let look = mark.look().expect("its look");
    assert_eq!(mark.label, Some((Arc::from("1."), INK, 8.)));
    assert_eq!(
        look.image,
        Some(MarkImage {
            image,
            fit: ObjectFit::Cover
        })
    );
    assert_eq!(look.align, Some(TextAlign::Center));
    assert_eq!(look.svg, Some(svg));
    assert_eq!(look.decoration, Some(decoration));
    assert_eq!(look.rotate, 90.);
    // As a literal's fields were: an option as it is, too.
    let none = mark
        .clone()
        .label(None)
        .image(None)
        .align(None)
        .svg(None)
        .decoration(None);
    assert!(none.label.is_none());
    let left = none.look().expect("its turn");
    assert!(left.image.is_none() && left.align.is_none());
    assert!(left.svg.is_none() && left.decoration.is_none());
    // A plain mark never carries a look, whatever else it has.
    let plain = LineMark::new(1., 2., 3., 4., INK)
        .bottom(5.)
        .radius(6.)
        .border_color(INK)
        .border_width(7.)
        .from_right(true)
        .mono(true)
        .tabular(true)
        .pinned(true)
        .over_text(true)
        .label((Arc::from("1."), INK, 8.));
    assert!(LineMark::default().look().is_none());
    assert!(plain.look().is_none());
    let nothing = plain.clone().svg(None).image(None).align(None);
    assert!(nothing.decoration(None).look().is_none(), "given nothing");
    let right = plain.align(Some(TextAlign::Right));
    assert_eq!(
        right.look().map(|look| look.align),
        Some(Some(TextAlign::Right))
    );
    assert_eq!(
        LineMark::new(1., 2., 3., 4., INK),
        LineMark::default()
            .x(1.)
            .y(2.)
            .width(3.)
            .height(4.)
            .fill(INK)
    );
}

#[test]
fn every_field_of_a_group_frame_has_a_builder() {
    let frame = GroupFrame::default().left(1.).width(2.).content(3.);
    assert_eq!(frame, GroupFrame::new(Some(1.), Some(2.), Some(3.)));
    assert_eq!(
        frame.left(None).width(Some(2.)).content(None),
        GroupFrame::new(None, Some(2.), None)
    );
}

/// Built by `new`, its builders, or whole (it gains no fields).
#[test]
fn a_marks_image_is_built_outside() {
    let image = |byte: u8| Arc::new(ImageData::new(1, 1, vec![byte; 4]).unwrap());
    let whole = MarkImage {
        image: image(9),
        fit: ObjectFit::Contain,
    };
    assert_eq!(whole.fit, MarkImage::new(image(9), ObjectFit::Contain).fit);
    let cover = MarkImage::cover(image(1));
    assert_eq!(cover.fit, ObjectFit::Cover);
    let other = image(2);
    let built = cover.fit(ObjectFit::Contain).image(other.clone());
    assert_eq!(built.fit, ObjectFit::Contain);
    assert!(Arc::ptr_eq(&built.image, &other));
}

/// Made on another thread, as before marks had looks.
#[test]
fn marks_rooms_and_frames_cross_threads() {
    const fn crosses<T: Send + Sync>() {}
    const {
        crosses::<LineMark>();
        crosses::<MarkLook>();
        crosses::<MarkImage>();
        crosses::<LineInset>();
        crosses::<GroupFrame>();
    }
}
