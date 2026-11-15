//! An icon in the text (`TextSpan::icon`) lands on whole device pixels as
//! the text beside it does (Blink paints images and SVG at
//! `ToPixelSnappedRect`, text at its line's rounded top), so the two keep
//! one offset as a scroll view moves them by fractions of a pixel: no
//! frame shows the icon a row off its text.
use std::sync::Arc;
use zgx::{
    compose::{SpanIcon, prelude::*},
    reactive::Signal,
    svg::SvgData,
    widgets::Ui,
};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##;
const SCALE: f32 = 2.;
const W: f32 = 200.;
const H: f32 = 100.;

fn paragraph(offset: Signal<f32>) -> View {
    paragraph_at(offset, 30.)
}

/// The paragraph `top` points down its scroll view.
fn paragraph_at(offset: Signal<f32>, top: f32) -> View {
    let icon = SpanIcon {
        svg: Arc::new(SvgData::new(SQUARE.as_bytes().to_vec()).unwrap()),
        size: 16.,
        gap: 4.,
        tint: None,
        bg: None,
        radius: 0.,
        pad: 0.,
    };
    scroll(offset).w(W).h(H).child(
        column().pt(top).pl(10.).child(
            rich_text()
                .child(text_span("HHHHH").icon(icon))
                .text_size(14.)
                .line_height(21.)
                .text_color(rgb(0x000000)),
        ),
    )
}

/// The first row (device pixels) any pixel of which `ink` holds.
fn first_row(pixels: &[u8], width: usize, ink: impl Fn(&[u8]) -> bool) -> Option<usize> {
    pixels
        .chunks(width * 4)
        .position(|row| row.chunks(4).any(&ink))
}

#[test]
fn an_icon_keeps_its_offset_to_its_text_as_a_scroll_moves_them_by_fractions() {
    let (w, h) = ((W * SCALE) as u32, (H * SCALE) as u32);
    let mut gpu = GpuRenderer::new(w, h).unwrap();
    gpu.set_scale_factor(SCALE);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut ui = Ui::new(W, H);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let offset = ui.signal(0.);
    ui.mount(paragraph(offset.clone()));
    let full = zgx::scene::Rect::new(0., 0., W, H);
    let mut offsets = Vec::new();
    // Eighths of a point: every fraction of a device pixel at 2x, twice.
    for step in 0..32 {
        let at = step as f32 / 8.;
        offset.set(at);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &[full]).unwrap();
        let pixels = gpu.readback().unwrap();
        let red = |p: &[u8]| p[0] > 200 && p[1] < 80 && p[2] < 80;
        let dark = |p: &[u8]| p[0] < 120 && p[1] < 120 && p[2] < 120;
        let icon = first_row(&pixels, w as usize, red).expect("the icon draws");
        let text = first_row(&pixels, w as usize, dark).expect("the text draws");
        offsets.push((at, text as i64 - icon as i64));
    }
    let first = offsets[0].1;
    assert!(
        offsets.iter().all(|(_, offset)| *offset == first),
        "text minus icon (device rows) by scroll offset: {offsets:?}"
    );
}

/// Where its box puts the icon on half a device pixel (its top 32.75 points
/// down at 2x: 65.5 pixels), it is drawn whole on the pixel below, as Blink
/// rounds an image's rect (half up), its top row fully red; elsewhere where
/// its box puts it.
#[test]
fn an_icon_on_half_a_device_pixel_draws_whole_on_the_pixel_below() {
    let (w, h) = ((W * SCALE) as u32, (H * SCALE) as u32);
    for (top, row) in [(30., 65), (30.25, 66), (30.5, 66), (30.75, 67)] {
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(SCALE);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(W, H);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let offset = ui.signal(0.);
        ui.mount(paragraph_at(offset, top));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &[zgx::scene::Rect::new(0., 0., W, H)])
            .unwrap();
        let pixels = gpu.readback().unwrap();
        let any_red = |p: &[u8]| p[0] > 60 && p[1] < p[0] / 2 && p[2] < p[0] / 2;
        let first = first_row(&pixels, w as usize, any_red).expect("the icon draws");
        let line = &pixels[first * w as usize * 4..(first + 1) * w as usize * 4];
        let reddest = line.chunks(4).map(|p| p[0]).max().unwrap();
        assert_eq!(
            (first, reddest),
            (row, 255),
            "paragraph at {top}: its icon's first row and how red it is"
        );
    }
}
