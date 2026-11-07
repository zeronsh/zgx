//! zgx's drawn glass (`zgx::glass::GlassStyle::drawn`) over a backdrop at 2x,
//! laid out as `scripts/glass_compare.py` captures the system's, for it to
//! compare pixel by pixel. `GLASS_SCENE`:
//! - `patches` (the default): a 1000 by 800 window of twenty solid patches,
//!   200 points square, each under an 80 point pane with corners of 16;
//! - `edge`: the same window black on its left half and white on its right,
//!   under one 600 by 400 pane at (200, 200);
//! - `picture`: the picture at `GLASS_PICTURE` (cut to fill the window), and
//!   `text`: lines of text, black on white or, dark, white on near black;
//!   each under one 440 by 280 pane at (280, 260) with corners of 24.
//!
//! `GLASS_PANES=0`: the backdrop alone, for the system's to lie over.
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
    let scene_name = std::env::var("GLASS_SCENE").unwrap_or_else(|_| "patches".into());
    let large = scene_name == "picture" || scene_name == "text";
    let style = GlassStyle {
        variant: match std::env::var_os("GLASS_CLEAR") {
            Some(_) => GlassVariant::Clear,
            None => GlassVariant::Regular,
        },
        tint: numbers("GLASS_TINT").map(|t| {
            let [r, g, b, a] = [t[0], t[1], t[2], t[3]].map(|v| (v * 255.).round() as u8);
            Color(r, g, b, a)
        }),
        radius: if large { 24. } else { 16. },
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
    gpu.install_text(&mut scene);
    let panes = if scene_name == "picture" {
        let path = std::env::var("GLASS_PICTURE").expect("GLASS_PICTURE: a picture's path");
        let picture = image::open(path).expect("a picture").to_rgba8();
        // Its middle, cut to the window's shape.
        let (w, h) = picture.dimensions();
        let (cut_w, cut_h) = (w.min(h * 5 / 4), h.min(w * 4 / 5));
        let cut =
            image::imageops::crop_imm(&picture, (w - cut_w) / 2, (h - cut_h) / 2, cut_w, cut_h)
                .to_image();
        let data = zgx::image::ImageData::new(cut_w, cut_h, cut.into_raw()).expect("its pixels");
        scene.append(
            root,
            NodeKind::Image(std::sync::Arc::new(data)),
            fixed(width, height),
        );
        vec![(280., 260., 440., 280.)]
    } else if scene_name == "text" {
        let (paper, ink) = match dark {
            true => (Color(28, 28, 30, 255), Color(240, 240, 245, 255)),
            false => (Color(255, 255, 255, 255), Color(20, 20, 24, 255)),
        };
        rect(&mut scene, root, (0., 0., width, height), paper);
        for line in 0..24 {
            let text = scene.append(
                root,
                NodeKind::Text {
                    text: format!(
                        "{line:02}  Glass takes on what lies behind it, and its words stay legible over anything."
                    )
                    .into(),
                    color: ink,
                    font_size: 22.,
                },
                Style::default(),
            );
            scene.set_transform(
                text,
                Transform {
                    x: 40.,
                    y: 20. + line as f32 * 32.,
                },
            );
        }
        vec![(280., 260., 440., 280.)]
    } else if scene_name == "edge" {
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
    let panes = match std::env::var("GLASS_PANES").is_ok_and(|n| n == "0") {
        true => Vec::new(),
        false => panes,
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
