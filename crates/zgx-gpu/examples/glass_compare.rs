//! zgx's drawn glass (`zgx::glass::GlassStyle::drawn`) over stripes, at 2x, the
//! layout `scripts/glass_compare.py` measures against the system's: a 420 by
//! 320 window, twenty 16-point stripes, a 300 by 200 pane at (60, 60), corners
//! of 24. `GLASS_DARK=1`: dark stripes. `GLASS_LIGHT=1`: a light appearance.
//! `GLASS_CLEAR=1`: the clear variant.
//! Parameters: `FROST`, `SATURATION`, `TINT` (amount), `TINT_GRAY`, 0-255.
//! Run: cargo run -p zgx-gpu --example glass_compare -- out.png
use zgx::glass::{GlassStyle, GlassVariant};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn env(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}
fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "glass.png".into());
    let dark = std::env::var_os("GLASS_DARK").is_some();
    let clear = std::env::var_os("GLASS_CLEAR").is_some();
    let palette = match dark {
        true => [
            Color(12, 14, 20, 255),
            Color(60, 20, 90, 255),
            Color(10, 70, 90, 255),
            Color(90, 30, 30, 255),
        ],
        false => [
            Color(242, 38, 38, 255),
            Color(26, 64, 242, 255),
            Color(245, 200, 60, 255),
            Color(255, 255, 255, 255),
        ],
    };
    let mut gpu = GpuRenderer::new(840, 640).expect("a renderer");
    gpu.set_scale_factor(2.);
    let mut scene = Scene::new(420., 320.);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for row in 0..20 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(palette[(19 - row) % 4]),
            fixed(420., 16.),
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: row as f32 * 16.,
            },
        );
    }
    let style = GlassStyle {
        variant: if clear {
            GlassVariant::Clear
        } else {
            GlassVariant::Regular
        },
        ..Default::default()
    };
    let mut drive = style.drawn(std::env::var_os("GLASS_LIGHT").is_none());
    if let Some(frost) = env("FROST") {
        drive.frost = frost;
    }
    if let Some(saturation) = env("SATURATION") {
        drive.params[0][2] = saturation;
    }
    if let Some(tint) = env("TINT") {
        drive.params[0][3] = tint;
    }
    if let Some(gray) = env("TINT_GRAY") {
        drive.params[1][..3].fill(gray / 255.);
    }
    let pane = scene.append(
        root,
        NodeKind::Quad(QuadStyle {
            radius: style.radius,
            ..Default::default()
        }),
        fixed(300., 200.),
    );
    scene.set_transform(pane, Transform { x: 60., y: 60. });
    drive.apply(&mut scene, pane, 0.);
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).expect("a frame");
    let pixels = gpu.readback().expect("its pixels");
    image::save_buffer(&out, &pixels, 840, 640, image::ExtendedColorType::Rgba8).expect("the file");
}
