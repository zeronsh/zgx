//! An inline box's character draws nothing, whatever its run's font holds:
//! in a font with no space and a visible `.notdef` (an icon font, say; this
//! one is DejaVu Sans cut down to its `A`), no missing-glyph box shows.
use zgx::{
    rich_text::{InlineBox, RichText, TextRun},
    scene::{Color, NodeKind, Rect, Scene, Style},
    text_layout::{FontFamily, FontStyle},
};
use zgx_gpu::{GpuRenderer, text::FontData};

const FONT: &[u8] = include_bytes!("fonts/NoSpace-subset.ttf");

#[test]
fn a_boxs_character_draws_nothing_in_a_font_without_a_space() {
    let Ok(mut gpu) = GpuRenderer::new(200, 60) else {
        eprintln!("no GPU adapter");
        return;
    };
    FontData::new(std::sync::Arc::<[u8]>::from(FONT))
        .expect("the font")
        .install(&mut gpu.text_system().borrow_mut());
    let mut scene = Scene::new(200., 60.);
    gpu.install_text(&mut scene);
    let font = FontStyle {
        family: FontFamily::Named("Zgx NoSpace".into()),
        ..Default::default()
    };
    let run = |range| TextRun {
        range,
        font: font.clone(),
        font_size: 24.,
        color: Color(255, 255, 255, 255),
        ..Default::default()
    };
    let rich = std::sync::Arc::new(
        RichText::new("A\u{FFFC}A", vec![run(0..1), run(1..4), run(4..5)])
            .unwrap()
            .with_inline_boxes(vec![InlineBox {
                width: 40.,
                height: 20.,
                ..InlineBox::new(1)
            }])
            .unwrap(),
    );
    let root = scene.root();
    let back = scene.append(
        root,
        NodeKind::Rect(Color(0, 0, 0, 255)),
        Style {
            width: Some(200.),
            height: Some(60.),
            absolute: true,
            ..Default::default()
        },
    );
    let _ = back;
    let node = scene.append(
        root,
        NodeKind::RichText { text: rich.clone() },
        Style::default(),
    );
    scene.flush();
    gpu.render(&scene, &[Rect::new(0., 0., 200., 60.)]).unwrap();
    let frame = gpu.readback().unwrap();
    let at = scene.bounds(node);
    let places = scene.shape_rich_text(&rich, None).inline_boxes();
    let [(_, x, top)] = places[..] else {
        panic!("one box: {places:?}")
    };
    let (x, top) = ((at.x + x) as usize, (at.y + top) as usize);
    // The `A`s drew: something else shows nothing for another reason.
    let lit = |x0: usize, x1: usize, y0: usize, y1: usize| {
        (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let p = (y * 200 + x) * 4;
                frame[p] > 40 || frame[p + 1] > 40 || frame[p + 2] > 40
            })
        })
    };
    assert!(lit(0, x, 0, 60), "the first A shows");
    assert!(
        !lit(x + 1, x + 39, top.saturating_sub(4), (top + 24).min(60)),
        "ink where the box's character sits"
    );
}
