//! A span with a border and a least width is an inline outlined box (CSS's
//! border and `min-width` on an inline-block): a keycap in a sentence. Its
//! outline takes room all round, the box is at least as wide as asked
//! (border and padding included) with its text centred, and it wraps with
//! the sentence.
use zgx::{compose::prelude::*, scene::Color, style::rgb, widgets::Ui};
use zgx_gpu::GpuRenderer;

const WIDTH: u32 = 300;
const HEIGHT: u32 = 100;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// "Press " then a keycap holding `key` (a red outline, its text black on the
/// white ground so only the outline is red), then " to go", at 2x.
fn draw(key: &str, least: f32, border: f32) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(WIDTH * 2, HEIGHT * 2).unwrap();
    gpu.set_scale_factor(2.);
    let mut ui = Ui::new(WIDTH as f32, HEIGHT as f32);
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.refresh_text_geometry();
    ui.mount(
        column()
            .size(WIDTH as f32, HEIGHT as f32)
            .p(10.)
            .bg(rgb(0xffffff))
            .child(
                rich_text()
                    .child(text_span("Above\nPress ").text_color(rgb(0x000000)))
                    .child({
                        let mut span = text_span(key)
                            .text_color(rgb(0x000000))
                            .padding(4., 3.)
                            .radius(7.5)
                            .border(border, Color(255, 0, 0, 255));
                        if least > 0. {
                            span = span.min_width(least);
                        }
                        span
                    })
                    .child(text_span(" to go\nBelow").text_color(rgb(0x000000)))
                    .w(280.)
                    .text_size(12.)
                    .line_height(20.),
            ),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let scene = ui.scene.borrow();
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap()
}

/// The box of the red outline: the leftmost and rightmost red columns, and
/// the top and bottom red rows, in points.
fn outline(pixels: &[u8]) -> Option<(f32, f32, f32, f32)> {
    let w = (WIDTH * 2) as usize;
    let red = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        pixels[i] > 200 && pixels[i + 1] < 90 && pixels[i + 2] < 90
    };
    let (mut l, mut t, mut r, mut b) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..(HEIGHT * 2) as usize {
        for x in 0..w {
            if red(x, y) {
                (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
            }
        }
    }
    (l != usize::MAX).then_some((
        l as f32 / 2.,
        t as f32 / 2.,
        (r + 1) as f32 / 2.,
        (b + 1) as f32 / 2.,
    ))
}

#[test]
fn a_bordered_span_draws_its_outline_and_takes_its_room() {
    let _gpu = gpu_fixture();
    let plain = outline(&draw("K", 0., 1.)).expect("an outline");
    let with_min = outline(&draw("K", 24., 1.)).expect("an outline");
    let thick = outline(&draw("K", 0., 3.)).expect("an outline");
    let (w, h) = (plain.2 - plain.0, plain.3 - plain.1);
    // The text, its padding (4 a side) and the border (1 a side).
    assert!(w > 10. && h > 14., "a box: {plain:?}");
    // A least width wider than that: the box is that wide, border-box.
    let wide = with_min.2 - with_min.0;
    assert!((wide - 24.).abs() < 1.1, "{wide} wide for a least of 24");
    // A thicker border takes more room, all round.
    let (tw, th) = (thick.2 - thick.0, thick.3 - thick.1);
    assert!((tw - w - 4.).abs() < 1.1, "{w} then {tw}");
    assert!((th - h - 4.).abs() < 1.1, "{h} then {th}");
    // The text stays in the middle of the wide box: black pixels either side
    // of its centre.
    let pixels = draw("K", 24., 1.);
    let cx = (with_min.0 + with_min.2) / 2.;
    let row = ((with_min.1 + with_min.3) / 2. * 2.) as usize;
    let dark = |x: f32| {
        let i = (row * (WIDTH * 2) as usize + (x * 2.) as usize) * 4;
        pixels[i] < 100
    };
    let span = |from: f32, to: f32| {
        let (from, to) = ((from * 2.) as i32, (to * 2.) as i32);
        (from..to).any(|x| dark(x as f32 / 2.))
    };
    let (left, right) = (
        span(with_min.0 + 1.5, cx - 0.5),
        span(cx - 0.5, with_min.2 - 1.5),
    );
    assert!(left || right, "the key draws inside its box");
    // Centred: equal room either side of the key's ink.
    let ink: Vec<i32> = ((with_min.0 * 2.) as i32 + 4..(with_min.2 * 2.) as i32 - 4)
        .filter(|x| dark(*x as f32 / 2.))
        .collect();
    let (first, last) = (
        *ink.first().unwrap() as f32 / 2.,
        *ink.last().unwrap() as f32 / 2. + 0.5,
    );
    let (before, after) = (first - with_min.0, with_min.2 - last);
    assert!(
        (before - after).abs() < 3.5,
        "{before} before the key, {after} after"
    );
}
