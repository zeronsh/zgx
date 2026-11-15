//! A span that is a box of its own (`TextSpan::inline_box`, CSS's
//! `display: inline-flex`): its line height and padding are its box, which
//! its line's box holds as CSS's does, each glyph's half-leading its own. A
//! badge that fits inside the paragraph's strut moves nothing; one taller
//! grows its line about the baseline. A plain padded span's padding stays
//! its background's alone (CSS's for an inline span).
use zgx::{compose::prelude::*, scene::Color, style::rgb, widgets::Ui};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const SCALE: f32 = 2.;
const W: f32 = 200.;
const H: f32 = 80.;

/// Rows (device pixels) of a paragraph at 14/21: "HHH " in white, then a
/// red span "12" at 13, padded 4 across and `pad` down, its line height
/// `line` and a box of its own if `boxed` (no span at all if `pad` is
/// negative). The paragraph's box is blue.
struct Rows {
    /// The H's ink, first and last rows: its top and its baseline.
    ink: (usize, usize),
    /// The paragraph's box.
    paragraph: (usize, usize),
    /// The badge's background.
    badge: Option<(usize, usize)>,
}

fn rows(pad: f32, line: f32, boxed: bool) -> Rows {
    let (w, h) = ((W * SCALE) as u32, (H * SCALE) as u32);
    let mut gpu = GpuRenderer::new(w, h).unwrap();
    gpu.set_scale_factor(SCALE);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let mut text = rich_text().child(text_span("HHH ").text_color(rgb(0xffffff)));
    if pad >= 0. {
        let badge = text_span("12")
            .text_size(13.)
            .line_height(line)
            .text_color(rgb(0xff0000))
            .background(Color(255, 0, 0, 255))
            .padding(4., pad);
        text = text.child(if boxed { badge.inline_box() } else { badge });
    }
    ui.mount(
        column().size(W, H).pt(20.).pl(10.).bg(rgb(0x000000)).child(
            column()
                .bg(rgb(0x0000ff))
                .child(text.text_size(14.).line_height(21.)),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &[zgx::scene::Rect::new(0., 0., W, H)])
        .unwrap();
    let pixels = gpu.readback().unwrap();
    let span = |ink: &dyn Fn(&[u8]) -> bool| {
        let rows: Vec<usize> = pixels
            .chunks(w as usize * 4)
            .enumerate()
            .filter(|(_, row)| row.chunks(4).any(ink))
            .map(|(y, _)| y)
            .collect();
        Some((*rows.first()?, *rows.last()?))
    };
    Rows {
        ink: span(&|p| p[0] > 200 && p[1] > 200 && p[2] > 200).expect("the text draws"),
        paragraph: span(&|p| p[2] > 200 && p[0] < 40 && p[1] < 40).expect("the paragraph draws"),
        // Any red: its edges may fall within a device pixel.
        badge: span(&|p| p[0] > 40 && p[1] < 40 && p[2] < p[0] / 2),
    }
}

#[test]
fn a_badge_inside_its_strut_moves_neither_the_baseline_nor_the_line() {
    let plain = rows(-1., 0., false);
    // At 13 points a line of 15 and 2 down is a box 19 tall, its reach about
    // the baseline within the 14-point text's at 21.
    let boxed = rows(2., 15., true);
    assert_eq!(boxed.ink, plain.ink, "the text where it was");
    assert_eq!(boxed.paragraph, plain.paragraph, "the line as tall");
    let badge = boxed.badge.expect("the badge draws");
    let tall = badge.1 - badge.0 + 1;
    assert!(
        (38..=39).contains(&tall),
        "its box: line and padding, {tall} rows"
    );
}

#[test]
fn a_badge_taller_than_its_line_grows_it_about_the_baseline() {
    let plain = rows(-1., 0., false);
    // A box 16 + 2 × 8 = 32 tall: past the 21-point line above and below.
    let boxed = rows(8., 16., true);
    let badge = boxed.badge.expect("the badge draws");
    let tall = badge.1 - badge.0 + 1;
    // Its edges within a device pixel each: as tall, or a row more.
    assert!(
        (64..=65).contains(&tall),
        "its box: line and padding, {tall} rows"
    );
    let near = |a: usize, b: usize| a.abs_diff(b) <= 1;
    assert!(
        near(boxed.paragraph.0, badge.0),
        "the line starts where the box does: {:?} {badge:?}",
        boxed.paragraph
    );
    assert!(
        near(boxed.paragraph.1, badge.1),
        "and ends where it does: {:?} {badge:?}",
        boxed.paragraph
    );
    // The baseline moves down by as much as the box reaches above the strut.
    let moved = boxed.ink.1 - plain.ink.1;
    assert_eq!(boxed.ink.0 - plain.ink.0, moved, "the text moves whole");
    assert!(moved > 0, "the text moved down: {moved}");
    assert_eq!(boxed.paragraph.0, plain.paragraph.0, "from the same top");
}

#[test]
fn a_plain_padded_span_keeps_its_line_as_it_was() {
    let plain = rows(-1., 0., false);
    let padded = rows(8., 16., false);
    assert_eq!(padded.ink, plain.ink, "the text where it was");
    assert_eq!(padded.paragraph, plain.paragraph, "the line as tall");
}
