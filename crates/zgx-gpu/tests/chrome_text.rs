//! Text on macOS at 2x draws as Chrome's does: Core Text's mask with font
//! smoothing, blended through Skia's coverage table by the text's
//! luminance, at the quarter pixel its origin falls in. The frame is
//! compared with Chrome's own pixels (`fixtures/chrome_text.png`, headless
//! Chrome 157 at a device scale of 2; four bands: Geist 400 at 14 px black
//! on white, 600 at 16 px navy on white at x 10.25, 400 at 14 px white on
//! near black, 500 at 13 px grey on charcoal at x 10.5): swash's outlines are
//! 13% lighter and 100 times further from it.
#![cfg(target_os = "macos")]
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const CHROME: &[u8] = include_bytes!("fixtures/chrome_text.png");

fn band(top: f32, bg: Color, fg: Color, size: f32, weight: u16, x: f32, text_: &str) -> View {
    div()
        .absolute()
        .ml(0.)
        .mt(top)
        .size(260., 30.)
        .bg(bg)
        .child(
            div().absolute().ml(x).mt(5.).w(240.).h(20.).child(
                text(text_)
                    .font_family("Geist")
                    .font_weight(weight)
                    .text_size(size)
                    .line_height(20.)
                    .text_color(fg),
            ),
        )
}

fn frame() -> Vec<u8> {
    let mut gpu = GpuRenderer::new(520, 240).unwrap();
    gpu.set_scale_factor(2.);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut ui = Ui::new(260., 120.);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let rgb = |r, g, b| Color(r, g, b, 255);
    ui.mount(
        div()
            .size(260., 120.)
            .child(band(
                0.,
                rgb(255, 255, 255),
                rgb(0, 0, 0),
                14.,
                400,
                10.,
                "Hello hamburgefonstiv, 0123",
            ))
            .child(band(
                30.,
                rgb(255, 255, 255),
                rgb(0x1a, 0x1a, 0x2e),
                16.,
                600,
                10.25,
                "Quick brown fox jumps.",
            ))
            .child(band(
                60.,
                rgb(0x10, 0x10, 0x14),
                rgb(255, 255, 255),
                14.,
                400,
                10.,
                "Hello hamburgefonstiv, 0123",
            ))
            .child(band(
                90.,
                rgb(0x1c, 0x1c, 0x22),
                rgb(0x9a, 0xa0, 0xb4),
                13.,
                500,
                10.5,
                "Quick brown fox jumps.",
            )),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    gpu.readback().unwrap()
}

#[test]
fn text_is_within_a_few_levels_of_chromes_pixels_light_and_dark() {
    let chrome = image::load_from_memory(CHROME).unwrap().to_rgb8();
    let ours = frame();
    if let Some(dir) = std::env::var_os("CHROME_TEXT_DUMP") {
        image::save_buffer(dir, &ours, 520, 240, image::ColorType::Rgba8).unwrap();
    }
    assert_eq!((chrome.width(), chrome.height()), (520, 240));
    for (band, name) in [
        "black on white",
        "navy 600 at x.25",
        "white on dark",
        "grey 500 at x.5",
    ]
    .iter()
    .enumerate()
    {
        let (mut sum, mut worst, mut off) = (0u64, 0u8, 0u32);
        for y in (band as u32 * 60)..(band as u32 * 60 + 60) {
            for x in 0..520u32 {
                let c = chrome.get_pixel(x, y).0;
                let o = &ours[((y * 520 + x) * 4) as usize..][..3];
                for k in 0..3 {
                    let d = c[k].abs_diff(o[k]);
                    sum += u64::from(d);
                    worst = worst.max(d);
                    off += u32::from(d > 6);
                }
            }
        }
        let mean = sum as f64 / (520. * 60. * 3.);
        assert!(
            mean < 0.35 && off < 40,
            "{name}: mean {mean:.3}, worst {worst}, {off} channels off by more than 6"
        );
    }
}
