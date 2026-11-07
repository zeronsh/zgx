//! A contact sheet of every ready-made effect and preset (`zgx_fx`), drawn
//! offscreen at one moment: `TIME` seconds (1.2), `SCALE` pixels a point (2),
//! `REDUCED=1` with motion reduced, `ONLY` names containing it. One-shots are
//! fired `AGE` seconds (0.12) before the frame. Writes `fx-sheet.png`, or the path given.
//! Run: cargo run --release -p zgx-fx --example fx_sheet -- out.png
use std::sync::Arc;
use std::time::{Duration, Instant};
use zgx::decoration::{Background, Decoration};
use zgx::scene::*;
use zgx_fx::{Fx, FxKind, catalog};
use zgx_gpu::GpuRenderer;

const CELL: (f32, f32) = (230., 180.);
const BOX: (f32, f32) = (150., 90.);
const COLUMNS: usize = 7;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
/// Stripes and a word: something to bend, split and frost.
fn backdrop(scene: &mut Scene, parent: NodeId, x: f32, y: f32) {
    let colors = [
        Color(38, 48, 88, 255),
        Color(214, 92, 72, 255),
        Color(240, 196, 92, 255),
        Color(58, 140, 150, 255),
    ];
    for i in 0..10 {
        let stripe = scene.append(
            parent,
            NodeKind::Rect(colors[i % colors.len()]),
            fixed(CELL.0 - 4., 14.),
        );
        scene.set_transform(
            stripe,
            Transform {
                x: x + 2.,
                y: y + 2. + i as f32 * 14.,
            },
        );
    }
}
#[allow(clippy::too_many_arguments)]
fn cell(
    scene: &mut Scene,
    parent: NodeId,
    name: &str,
    fx: &Fx,
    at: (f32, f32),
    now: Instant,
    time: f32,
    reduced: bool,
) {
    backdrop(scene, parent, at.0, at.1);
    let (x, y) = (at.0 + (CELL.0 - BOX.0) * 0.5, at.1 + 28.);
    let quad = QuadStyle {
        fill: Color(24, 28, 40, if fx.kind() == FxKind::Fill { 255 } else { 0 }),
        radius: 16.,
        decoration: match fx.kind() {
            FxKind::Fill => Some(Arc::new(Decoration {
                background: Some(Background::Shader(fx.shader())),
                ..Default::default()
            })),
            FxKind::Layer => None,
        },
        ..Default::default()
    };
    let kind = NodeKind::Panel {
        layout: Layout::Overlay,
        quad,
    };
    let id = scene.append(parent, kind, fixed(BOX.0, BOX.1));
    scene.set_transform(id, Transform { x, y });
    fx.apply(scene, id, time, reduced, now);
    // Content, for those working on what's painted.
    if fx.kind() == FxKind::Layer {
        let word = scene.append(
            id,
            NodeKind::Text {
                text: "Sample".into(),
                color: Color(255, 255, 255, 255),
                font_size: 26.,
            },
            Style::default(),
        );
        scene.set_transform(word, Transform { x: 34., y: 26. });
    }
    let label = scene.append(
        parent,
        NodeKind::Text {
            text: name.into(),
            color: Color(255, 255, 255, 255),
            font_size: 12.,
        },
        Style::default(),
    );
    scene.set_transform(
        label,
        Transform {
            x: at.0 + 6.,
            y: at.1 + CELL.1 - 22.,
        },
    );
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fx-sheet.png".into());
    let time: f32 = env("TIME", 1.2);
    let scale: f32 = env("SCALE", 2.);
    let reduced = env("REDUCED", 0) != 0;
    let only = std::env::var("ONLY").unwrap_or_default();
    let all: Vec<(String, Fx)> = catalog()
        .into_iter()
        .filter(|(name, _)| only.is_empty() || only.split(',').any(|word| name.contains(word)))
        .collect();
    let rows = all.len().div_ceil(COLUMNS);
    let size = (COLUMNS as f32 * CELL.0, rows as f32 * CELL.1);
    let (width, height) = ((size.0 * scale) as u32, (size.1 * scale) as u32);
    let mut gpu = GpuRenderer::new(width, height).expect("a renderer");
    gpu.set_scale_factor(scale);
    gpu.set_background(Color(12, 14, 20, 255));
    let mut scene = Scene::new(size.0, size.1);
    let fonts = gpu.text_system();
    scene.set_text_measurer(move |text: &str, size: f32, width: Option<f32>| {
        zgx_gpu::measure_text(&mut fonts.borrow_mut(), text, size, width)
    });
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    // One-shots first drew `time` ago.
    let start = Instant::now();
    let now = start;
    for (index, (name, fx)) in all.iter().enumerate() {
        let at = (
            (index % COLUMNS) as f32 * CELL.0,
            (index / COLUMNS) as f32 * CELL.1,
        );
        // Start each shot now (ages at 0), then draw them part way through.
        let _ = fx.uniforms(0., reduced, now);
        let age = now + Duration::from_secs_f32(env("AGE", 0.12));
        cell(&mut scene, root, name, fx, at, age, time, reduced);
    }
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).expect("a frame");
    // A second pass: pipelines compile off the frame.
    std::thread::sleep(Duration::from_millis(1500));
    gpu.render(&scene, &[Rect::new(0., 0., size.0, size.1)])
        .expect("a frame");
    let pixels = gpu.readback().expect("its pixels");
    image::save_buffer(
        &out,
        &pixels,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .expect("the file");
    println!("{out}: {} effects", all.len());
}
