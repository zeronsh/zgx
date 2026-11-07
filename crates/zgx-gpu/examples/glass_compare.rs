//! zgx's drawn glass (`zgx::glass::GlassStyle::drawn`) over a backdrop at 2x,
//! laid out as `scripts/glass_compare.py` captures the system's, for it to
//! compare pixel by pixel. `GLASS_SCENE`:
//! - `patches` (the default): a 1000 by 800 window of twenty solid patches,
//!   200 points square, each under an 80 point pane with corners of 16;
//! - `edge`: the same window black on its left half and white on its right,
//!   under one 600 by 400 pane at (200, 200).
//!
//! `GLASS_DARK=1`: a dark appearance. `GLASS_CLEAR=1`: the clear variant.
//! `GLASS_TINT=r,g,b,a` (0-1): a tint. To try values: `FROST`, and `PARAMS`,
//! sixteen numbers in place of the material's.
//! Run: cargo run -p zgx-gpu --example glass_compare -- out.png
use zgx::glass::{GlassStyle, GlassVariant};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

/// The patches' colours, from the bottom left, row by row.
const PATCHES: [(u8, u8, u8); 20] = [
    (0, 0, 0),
    (64, 64, 64),
    (128, 128, 128),
    (192, 192, 192),
    (255, 255, 255),
    (255, 0, 0),
    (0, 255, 0),
    (0, 0, 255),
    (255, 255, 0),
    (0, 255, 255),
    (255, 0, 255),
    (128, 0, 0),
    (0, 128, 0),
    (0, 0, 128),
    (255, 128, 0),
    (40, 90, 160),
    (200, 60, 90),
    (90, 200, 120),
    (30, 30, 40),
    (240, 230, 200),
];

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn numbers(name: &str) -> Option<Vec<f32>> {
    let value = std::env::var(name).ok()?;
    Some(
        value
            .split(',')
            .map(|v| v.trim().parse().expect("a number"))
            .collect(),
    )
}
fn rect(scene: &mut Scene, parent: NodeId, (x, y, w, h): (f32, f32, f32, f32), color: Color) {
    let node = scene.append(parent, NodeKind::Rect(color), fixed(w, h));
    scene.set_transform(node, Transform { x, y });
}
fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "glass.png".into());
    let dark = std::env::var_os("GLASS_DARK").is_some();
    let edge = std::env::var("GLASS_SCENE").is_ok_and(|scene| scene == "edge");
    let style = GlassStyle {
        variant: match std::env::var_os("GLASS_CLEAR") {
            Some(_) => GlassVariant::Clear,
            None => GlassVariant::Regular,
        },
        tint: numbers("GLASS_TINT").map(|t| {
            let [r, g, b, a] = [t[0], t[1], t[2], t[3]].map(|v| (v * 255.).round() as u8);
            Color(r, g, b, a)
        }),
        radius: 16.,
        ..Default::default()
    };
    let mut drive = style.drawn(dark);
    if let Some(frost) = numbers("FROST") {
        drive.frost = frost[0];
    }
    if let Some(params) = numbers("PARAMS") {
        for (index, value) in params.into_iter().enumerate() {
            drive.params[index / 4][index % 4] = value;
        }
    }
    let (width, height) = (1000., 800.);
    let mut gpu = GpuRenderer::new(2000, 1600).expect("a renderer");
    gpu.set_scale_factor(2.);
    let mut scene = Scene::new(width, height);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let panes = if edge {
        rect(
            &mut scene,
            root,
            (0., 0., 500., height),
            Color(0, 0, 0, 255),
        );
        rect(
            &mut scene,
            root,
            (500., 0., 500., height),
            Color(255, 255, 255, 255),
        );
        vec![(200., 200., 600., 400.)]
    } else {
        let mut panes = Vec::new();
        for (index, (r, g, b)) in PATCHES.into_iter().enumerate() {
            let x = (index % 5) as f32 * 200.;
            let y = height - (index / 5 + 1) as f32 * 200.;
            rect(&mut scene, root, (x, y, 200., 200.), Color(r, g, b, 255));
            panes.push((x + 60., y + 60., 80., 80.));
        }
        panes
    };
    for (x, y, w, h) in panes {
        let pane = scene.append(
            root,
            NodeKind::Quad(QuadStyle {
                radius: style.radius,
                shadow: style.drawn_shadow(dark),
                ..Default::default()
            }),
            fixed(w, h),
        );
        scene.set_transform(pane, Transform { x, y });
        drive.apply(&mut scene, pane, 0.);
    }
    scene.flush();
    gpu.render(&scene, &[Rect::new(0., 0., width, height)])
        .expect("a frame");
    let pixels = gpu.readback().expect("its pixels");
    image::save_buffer(&out, &pixels, 2000, 1600, image::ExtendedColorType::Rgba8)
        .expect("the file");
}
