//! Rounded span backgrounds (`TextSpan::radius`, an inline code chip) round
//! on their own corners, wherever they sit in their text, and a span
//! wrapped across lines rounds each line's piece on its own ends (CSS's
//! `box-decoration-break: clone`).
use zgx::{compose::prelude::*, scene::Color, style::rgb, widgets::Ui};
use zgx_gpu::GpuRenderer;

const WIDTH: u32 = 300;
const HEIGHT: u32 = 160;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A text `width` wide: "before ", a red chip of `chip`, " after", white on
/// black.
fn draw(chip: &str, width: f32) -> Vec<u8> {
    let mut gpu = GpuRenderer::new(WIDTH, HEIGHT).unwrap();
    let mut ui = Ui::new(WIDTH as f32, HEIGHT as f32);
    gpu.install_text(&mut ui.scene.borrow_mut());
    ui.refresh_text_geometry();
    ui.mount(
        column()
            .size(WIDTH as f32, HEIGHT as f32)
            .p(10.)
            .bg(rgb(0x000000))
            .child(
                rich_text()
                    .child(text_span("before ").text_color(rgb(0xffffff)))
                    .child(
                        text_span(chip)
                            .text_color(rgb(0x000000))
                            .background(Color(255, 0, 0, 255))
                            .padding(4., 3.)
                            .radius(8.),
                    )
                    .child(text_span(" after").text_color(rgb(0xffffff)))
                    .w(width)
                    .text_wrap(true)
                    .text_size(20.)
                    .line_height(34.),
            ),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let scene = ui.scene.borrow();
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap()
}

/// Each run of rows holding red (a chip's piece on a line): its box.
fn pieces(pixels: &[u8]) -> Vec<(u32, u32, u32, u32)> {
    let red = |x: u32, y: u32| {
        let i = ((y * WIDTH + x) * 4) as usize;
        pixels[i] > 200 && pixels[i + 1] < 40 && pixels[i + 2] < 40
    };
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut open = false;
    for y in 0..HEIGHT {
        let row: Vec<u32> = (0..WIDTH).filter(|&x| red(x, y)).collect();
        match (row.first(), row.last()) {
            (Some(&left), Some(&right)) => {
                if open {
                    let b = boxes.last_mut().unwrap();
                    *b = (b.0.min(left), b.1, b.2.max(right), y);
                } else {
                    boxes.push((left, y, right, y));
                    open = true;
                }
            }
            _ => open = false,
        }
    }
    boxes
}

/// Its corners rounded off (not red), its edges' middles red.
fn assert_rounded(pixels: &[u8], (left, top, right, bottom): (u32, u32, u32, u32), what: &str) {
    let red = |x: u32, y: u32| {
        let i = ((y * WIDTH + x) * 4) as usize;
        pixels[i] > 200 && pixels[i + 1] < 40 && pixels[i + 2] < 40
    };
    for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)] {
        assert!(!red(x, y), "{what}: the corner at ({x}, {y}) is square");
    }
    let (mx, my) = ((left + right) / 2, (top + bottom) / 2);
    for (x, y) in [(mx, top), (mx, bottom), (left, my), (right, my)] {
        assert!(red(x, y), "{what}: the edge at ({x}, {y}) is missing");
    }
}

#[test]
fn a_chip_inside_its_text_rounds_its_own_corners() {
    let _gpu = gpu_fixture();
    let pixels = draw("code", 280.);
    let found = pieces(&pixels);
    assert_eq!(found.len(), 1, "one chip: {found:?}");
    assert_rounded(&pixels, found[0], "a chip mid-line");
}

#[test]
fn a_wrapped_chip_rounds_each_lines_piece_on_its_own_ends() {
    let _gpu = gpu_fixture();
    let pixels = draw("a long chip that wraps", 150.);
    let found = pieces(&pixels);
    assert!(found.len() >= 2, "wrapped onto two lines: {found:?}");
    for (index, piece) in found.into_iter().enumerate() {
        assert_rounded(&pixels, piece, &format!("piece {index}"));
    }
}
