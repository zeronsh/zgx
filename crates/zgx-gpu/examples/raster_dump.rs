//! Draws the raster-floor probe lines (see `comparisons/raster`) with the
//! real renderer at 2x and writes the frame as a PNG: `raster_dump out.png`.
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("../tests/fonts/GeistVariable-subset.ttf");
const DEFAULT_TEXT: &str = "Hello hamburgefonstiv. The quick brown fox";

fn main() {
    let out = std::env::args().nth(1).expect("an output path");
    // Optional: family, weight, size, text (one black line on white).
    let family = std::env::args().nth(2);
    let weight: u16 = std::env::args().nth(3).map_or(400, |w| w.parse().unwrap());
    let size: f32 = std::env::args().nth(4).map_or(14., |w| w.parse().unwrap());
    let text_arg = std::env::args().nth(5);
    let text_line = text_arg.as_deref().unwrap_or(DEFAULT_TEXT);
    let (w, h) = (400u32, 140u32);
    let mut gpu = GpuRenderer::new(w * 2, h * 2).unwrap();
    gpu.set_scale_factor(2.);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let sf = gpu
        .text_system()
        .borrow()
        .db()
        .faces()
        .find(|f| f.post_script_name.starts_with(".SFNS-"))
        .and_then(|f| f.families.first().map(|(n, _)| n.clone()));
    let family = match (family.as_deref(), sf) {
        (Some("sf"), Some(name)) => Some(name),
        _ => family,
    };
    let mut ui = Ui::new(w as f32, h as f32);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let line = |x: f32, y: f32, fg: Color, bg: Option<Color>| {
        let t = text(text_line)
            .font_family(family.as_deref().unwrap_or("Geist"))
            .font_weight(weight)
            .text_size(size)
            .line_height(20.)
            .text_color(fg);
        let d = div().absolute().ml(x).mt(y).w(380.).h(20.).child(t);
        match bg {
            Some(bg) => d.bg(bg),
            None => d,
        }
    };
    let black = Color(0, 0, 0, 255);
    let white = Color(255, 255, 255, 255);
    ui.mount(
        div()
            .size(w as f32, h as f32)
            .bg(white)
            .child(line(10., 10., black, None))
            .child(line(10.25, 40., black, None))
            .child(line(10.5, 70., black, None))
            .child(line(10., 100., white, Some(black))),
    );
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    gpu.render(&ui.scene.borrow(), &damage).unwrap();
    let px = gpu.readback().unwrap();
    image::save_buffer(&out, &px, w * 2, h * 2, image::ColorType::Rgba8).unwrap();
}
