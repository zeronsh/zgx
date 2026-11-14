//! A text area's line with a picture in its room above it (`LineMark::image`), as a note's image over its caption:
//! the picture in its box, cropped to fill it, and the caption below it, aligned (`LineInset::align`) and editable.
use std::{rc::Rc, sync::Arc};
use zgx::{
    compose::prelude::*,
    image::ImageData,
    input::InputEvent,
    scene::{Color, NodeKind, Rect},
    text_layout::{FallbackTextLayout, TextAlign, TextLayout},
    widgets::{EditorHandle, LineInset, LineMark, LineSpans, MarkImage, SpanStyle, Ui},
};

const TEXT: &str = "intro\ncaption\nafter";
/// The caption's line: 120 points of room above it, its text centred.
const CAPTION: usize = 1;

fn note(picture: Arc<ImageData>) -> (Ui, EditorHandle) {
    note_with(picture, false)
}
/// The caption's word in a colour: a highlighter's span that colours, as a note's link does.
fn colour_caption(line: &str, _: u32) -> (LineSpans, u32) {
    let spans = match line.find("caption") {
        Some(at) => vec![(at..at + 7, SpanStyle::from(Color(0, 90, 200, 255)))],
        None => Vec::new(),
    };
    (spans, 0)
}
/// `note`, its caption coloured by a highlighter if `coloured`.
fn note_with(picture: Arc<ImageData>, coloured: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(500., 400.);
    let value = ui.signal(TEXT.to_string());
    let area = text_area("Note", value)
        .w(400.)
        .h(300.)
        .p(0.)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true);
    let view = ui.mount(if coloured {
        area.span_highlighter(colour_caption)
    } else {
        area
    });
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|i| match i {
        CAPTION => LineInset {
            above: 120.,
            align: Some(TextAlign::Center),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(move |i| match i {
        // 200 by 100, centred across the line, 10 points into its room.
        CAPTION => vec![LineMark {
            y: -110.,
            width: 200.,
            height: 100.,
            radius: 8.,
            align: Some(TextAlign::Center),
            image: Some(MarkImage::cover(picture.clone())),
            ..LineMark::default()
        }],
        _ => Vec::new(),
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    (ui, editor)
}
/// The picture's node and its box (the node clipping it), in the window.
fn picture(ui: &Ui, editor: &EditorHandle) -> (Rect, Rect, bool, f32) {
    let scene = ui.scene.borrow();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Image(_) = scene.kind(id) {
            let frame = scene.parent(id).expect("the picture's box");
            let radius = match scene.kind(frame) {
                NodeKind::Panel { quad, .. } => quad.radius,
                other => panic!("the picture's box is a panel, not {other:?}"),
            };
            return (
                scene.transformed_bounds(id),
                scene.transformed_bounds(frame),
                scene.style(frame).clip,
                radius,
            );
        }
        pending.extend(scene.children(id).iter().copied());
    }
    panic!("no picture drawn")
}
fn caret_x(ui: &Ui, editor: &EditorHandle, at: usize) -> f32 {
    editor.set_selection(at, at);
    ui.prepare_frame();
    ui.scene.borrow().transformed_bounds(editor.caret).x
}

#[test]
fn a_picture_fills_its_box_in_the_lines_room_above_a_centred_editable_caption() {
    // A square picture in a 2:1 box: cover scales it to the box's width and crops its height.
    let pixels = vec![200_u8; 50 * 50 * 4];
    let (mut ui, editor) = note(Arc::new(ImageData::new(50, 50, pixels).unwrap()));
    let origin = caret_x(&ui, &editor, 0);
    let top = ui.scene.borrow().transformed_bounds(editor.node).y;
    let (image, frame, clips, radius) = picture(&ui, &editor);
    // The caption's line starts at 20 (below "intro"), its room 120 tall, the box 10 into it.
    assert_eq!(
        frame,
        Rect::new(origin + 100., top + 30., 200., 100.),
        "centred, in the room"
    );
    assert!(clips, "the box clips what the picture crops");
    assert_eq!(radius, 8., "its corners rounded");
    assert_eq!(
        image,
        Rect::new(origin + 100., top + 30. - 50., 200., 200.),
        "200 square, centred on the box"
    );
    // The caption's text below the room, centred: 7 letters of 6 points in 400.
    let caption = TEXT.find("caption").unwrap();
    assert_eq!(
        caret_x(&ui, &editor, caption) - origin,
        179.,
        "before the centred caption"
    );
    // And editable: typed into, it reads and shows the letter.
    editor.set_selection(caption, caption);
    ui.prepare_frame();
    ui.dispatch(InputEvent::Text("A ".into()));
    ui.prepare_frame();
    assert_eq!(editor.value().get(), "intro\nA caption\nafter");
    assert_eq!(
        caret_x(&ui, &editor, caption) - origin,
        (400. - 9. * 6.) / 2.,
        "centred again at its new width"
    );
    let (_, frame, ..) = picture(&ui, &editor);
    assert_eq!(frame.y, top + 30., "the picture where it was");
}

/// Where the line holding `text` paints its first glyph, as a renderer lays out its paint item: the item's text
/// in its box, in its font (a styled line's runs carry theirs).
fn painted_x(ui: &Ui, text: &str) -> f32 {
    let scene = ui.scene.borrow();
    for item in scene.paint_items() {
        let width = Some(item.bounds.width);
        let laid: Box<dyn TextLayout> = match item.kind {
            NodeKind::Text {
                text: shown,
                font_size,
                ..
            } if shown.contains(text) => Box::new(FallbackTextLayout::with_font(
                shown, *font_size, width, item.font,
            )),
            NodeKind::RichText { text: shown } if shown.text().contains(text) => {
                Box::new(FallbackTextLayout::with_rich(shown, width))
            }
            _ => continue,
        };
        return item.bounds.x + laid.caret(0).x;
    }
    panic!("no line holding {text:?} paints")
}

#[test]
fn a_centred_caption_paints_where_its_caret_is_plain_and_coloured() {
    for coloured in [false, true] {
        let pixels = vec![200_u8; 4 * 4 * 4];
        let (ui, editor) = note_with(Arc::new(ImageData::new(4, 4, pixels).unwrap()), coloured);
        let origin = caret_x(&ui, &editor, 0);
        let caption = TEXT.find("caption").unwrap();
        let caret = caret_x(&ui, &editor, caption);
        assert_eq!(
            caret - origin,
            179.,
            "the caret centred (coloured: {coloured})"
        );
        assert_eq!(
            painted_x(&ui, "caption"),
            caret,
            "its glyphs there too (coloured: {coloured})"
        );
        // An unaligned line paints at its left, as before.
        assert_eq!(painted_x(&ui, "intro"), origin);
    }
}
