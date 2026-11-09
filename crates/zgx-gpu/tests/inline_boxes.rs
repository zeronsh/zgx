//! Inline boxes through the native shaper and the renderer: placed from the
//! lines drawn, aligned by real baselines, their characters drawing nothing.
use zgx::{
    compose::prelude::*,
    rich_text::InlineAlign,
    scene::{Color, NodeKind, Rect},
    widgets::Ui,
};
use zgx_gpu::{GpuRenderer, TextServices};

/// A chip takes the text's real advance before it, and its label's baseline
/// is the line's: the text around it reads straight through.
#[test]
fn a_chip_sits_where_the_shaper_leaves_room_with_its_label_on_the_baseline() {
    let services = TextServices::new();
    let mut ui = Ui::new(600., 200.);
    services.install(&mut ui.scene.borrow_mut());
    let handle = ui.mount(
        rich_text()
            .id("text")
            .w(500.)
            .text_size(15.)
            .text_wrap(true)
            .child(text_span("Ask "))
            .inline(
                row()
                    .id("chip")
                    .p(3.)
                    .child(text("label").id("label").text_size(15.)),
            )
            .child(text_span(" now"))
            .into(),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let node = scene.children(handle.find("text").unwrap())[0];
    let NodeKind::RichText { text } = scene.kind(node) else {
        panic!("a paragraph")
    };
    let at = scene.bounds(node);
    let chip = scene.bounds(handle.find("chip").unwrap());
    let label = scene.bounds(handle.find("label").unwrap());
    let layout = scene.shape_rich_text(text, Some(at.width));
    let [(offset, x, top)] = layout.inline_boxes()[..] else {
        panic!("one box")
    };
    assert_eq!(offset, "Ask ".len());
    assert!((chip.x - at.x - x).abs() < 0.01, "{chip:?} {x}");
    assert!((chip.y - at.y - top).abs() < 0.01, "{chip:?} {top}");
    assert!((x - layout.caret(offset).x).abs() < 0.01);
    assert!(x > 15.);
    // The text after it moved over by the chip.
    assert!((layout.caret(offset + 3).x - x - chip.width).abs() < 0.01);
    // Its label's baseline, 3 down its padding, is the box's; the line's.
    let b = text.inline_boxes()[0];
    assert!((label.y - chip.y - 3.).abs() < 0.01, "{label:?} {chip:?}");
    assert!(b.baseline > 3. && b.baseline < chip.height, "{b:?}");
    // The paragraph is one line, grown by the chip's padding only.
    let plain = services.text_cache().borrow_mut().measure(
        &mut services.text_system().borrow_mut(),
        &zgx::rich_text::RichText::new(
            "Ask",
            vec![zgx::rich_text::TextRun {
                range: 0..3,
                font_size: 15.,
                font: text.runs()[0].font.clone(),
                ..Default::default()
            }],
        )
        .unwrap(),
        None,
    );
    assert!(
        (at.height - (plain.1 + 6.)).abs() < 0.01,
        "{at:?} {plain:?}"
    );
}

/// A truncated paragraph lays out the boxes it keeps, and not those cut off.
#[test]
fn a_clamped_paragraph_lays_out_the_boxes_it_keeps() {
    let services = TextServices::new();
    let mut scene = zgx::scene::Scene::new(400., 200.);
    services.install(&mut scene);
    let font = zgx::text_layout::FontStyle::default();
    let run = |range| zgx::rich_text::TextRun {
        range,
        font: font.clone(),
        font_size: 15.,
        ..Default::default()
    };
    let text = "a\u{FFFC} bbbbbbbbbbbbbbbb \u{FFFC}";
    let end = text.len();
    let rich = zgx::rich_text::RichText::new(
        text,
        vec![run(0..1), run(1..4), run(4..end - 3), run(end - 3..end)],
    )
    .unwrap()
    .with_inline_boxes(vec![
        zgx::rich_text::InlineBox {
            width: 20.,
            height: 10.,
            baseline: 10.,
            ..zgx::rich_text::InlineBox::new(1)
        },
        zgx::rich_text::InlineBox {
            width: 20.,
            height: 10.,
            baseline: 10.,
            ..zgx::rich_text::InlineBox::new(end - 3)
        },
    ])
    .unwrap()
    .with_options(zgx::text_layout::TextOptions {
        overflow: zgx::text_layout::TextOverflow::Ellipsis,
        line_clamp: std::num::NonZeroU32::new(1),
    });
    let boxes = scene.shape_rich_text(&rich, Some(120.)).inline_boxes();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!(boxes[0].0, 1);
}

/// The box's character draws nothing of its own (no missing-glyph box), and
/// what the box holds draws in its place, over the text's background.
#[test]
fn a_boxs_character_draws_nothing_and_what_it_holds_draws_there() {
    let Ok(mut gpu) = GpuRenderer::new(300, 80) else {
        eprintln!("no GPU adapter");
        return;
    };
    let mut ui = Ui::new(300., 80.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let red = Color(255, 0, 0, 255);
    let handle = ui.mount(
        column().w_full().h_full().bg(Color(0, 0, 0, 255)).child(
            rich_text()
                .id("text")
                .w(280.)
                .text_size(16.)
                .text_color(Color(255, 255, 255, 255))
                .text_wrap(true)
                .child(text_span("ab"))
                .inline(inline_box(row().id("empty").w(30.).h(12.)).align(InlineAlign::Middle))
                .child(text_span("cd"))
                .inline(row().id("filled").w(30.).h(12.).bg(red)),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let scene = ui.scene.borrow();
    gpu.render(&scene, &[Rect::new(0., 0., 300., 80.)]).unwrap();
    let frame = gpu.readback().unwrap();
    let pixel = |x: f32, y: f32| {
        let at = (y as usize * 300 + x as usize) * 4;
        (frame[at], frame[at + 1], frame[at + 2])
    };
    let inside = |rect: Rect, mut f: Box<dyn FnMut((u8, u8, u8)) -> bool + '_>| {
        (rect.y as usize + 1..(rect.y + rect.height) as usize - 1).all(|y| {
            (rect.x as usize + 1..(rect.x + rect.width) as usize - 1)
                .all(|x| f(pixel(x as f32, y as f32)))
        })
    };
    let empty = scene.bounds(handle.find("empty").unwrap());
    let filled = scene.bounds(handle.find("filled").unwrap());
    assert!(
        inside(empty, Box::new(|p| p == (0, 0, 0))),
        "ink in an empty box"
    );
    assert!(
        inside(filled, Box::new(|p| p == (255, 0, 0))),
        "the box's fill"
    );
}
