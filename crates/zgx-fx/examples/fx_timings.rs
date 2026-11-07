//! What each ready-made effect (`zgx_fx`) costs the GPU, on this machine: ten
//! of it stacked over 960,000 pixels each (600 by 400 points at 2x), a frame
//! waited for before the next, less the same frames with ten plain boxes in
//! their place on the same path, a tenth of that. Damage, copies and passes
//! are the renderer's, as a window's; encoding is in it, and small beside the
//! GPU's work. A busy machine varies by half again: read them as relative costs.
//! `ONLY` names containing it, `FRAMES` (30), `RUNS` (3, their median).
//! Run: cargo run --release -p zgx-fx --example fx_timings
use std::sync::Arc;
use std::time::{Duration, Instant};
use zgx::decoration::{Background, Decoration};
use zgx::scene::*;
use zgx_fx::{Fx, FxKind, catalog};
use zgx_gpu::GpuRenderer;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
const STACKED: usize = 10;
struct Setting {
    gpu: GpuRenderer,
    scene: Scene,
    nodes: Vec<NodeId>,
}
fn setting(fx: Option<&Fx>) -> Setting {
    let mut gpu = GpuRenderer::new(1600, 1200).expect("a renderer");
    gpu.set_scale_factor(2.);
    // As frames with a layer shader always are.
    gpu.set_direct_metal(false);
    gpu.set_background(Color(12, 14, 20, 255));
    let mut scene = Scene::new(800., 600.);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..30 {
        let stripe = scene.append(
            root,
            NodeKind::Rect(Color(40 + i * 6, 60, 200 - i * 5, 255)),
            fixed(800., 20.),
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: f32::from(i) * 20.,
            },
        );
    }
    let filled = fx.filter(|fx| fx.kind() == FxKind::Fill);
    let quad = QuadStyle {
        fill: Color(255, 255, 255, 20),
        radius: 16.,
        decoration: filled.map(|fx| {
            Arc::new(Decoration {
                background: Some(Background::Shader(fx.shader())),
                ..Default::default()
            })
        }),
        ..Default::default()
    };
    let nodes = (0..STACKED)
        .map(|_| {
            let node = scene.append(
                root,
                NodeKind::Panel {
                    layout: Layout::Overlay,
                    quad: quad.clone(),
                },
                fixed(600., 400.),
            );
            scene.set_transform(node, Transform { x: 100., y: 100. });
            node
        })
        .collect();
    Setting { gpu, scene, nodes }
}
/// Seconds a frame.
fn run(fx: Option<&Fx>, frames: u32) -> f64 {
    let Setting {
        mut gpu,
        mut scene,
        nodes,
    } = setting(fx);
    let start = Instant::now();
    let mut step = |frame: u32| {
        let now = start + Duration::from_millis(u64::from(frame % 8) * 30);
        for &node in &nodes {
            match fx {
                Some(fx) => {
                    if frame.is_multiple_of(8) {
                        fx.fire(300., 200.);
                    }
                    fx.apply(&mut scene, node, frame as f32 / 60., false, now);
                }
                // The same box, its pixels drawn again each frame.
                None => scene.set_kind(
                    node,
                    NodeKind::Panel {
                        layout: Layout::Overlay,
                        quad: QuadStyle {
                            fill: Color(255, 255, 255, (frame % 40) as u8),
                            radius: 16.,
                            ..Default::default()
                        },
                    },
                ),
            }
        }
        let report = scene.flush();
        gpu.render(&scene, &report.damage).expect("a frame");
        gpu.readback().expect("its pixels");
    };
    // Warm: pipelines, layers and copies made.
    for frame in 0..3 {
        step(frame);
    }
    let times: Vec<f64> = (3..3 + frames)
        .map(|frame| {
            let began = Instant::now();
            step(frame);
            began.elapsed().as_secs_f64()
        })
        .collect();
    median(times)
}
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let frames = env("FRAMES", 30);
    let runs = env("RUNS", 3);
    let only = std::env::var("ONLY").unwrap_or_default();
    let base = median((0..runs).map(|_| run(None, frames)).collect());
    println!("| effect | px | backdrop | GPU ms a frame |\n| --- | ---: | :-: | ---: |");
    println!(
        "| (ten plain boxes) | 960000 | | {:.2} a frame in all |",
        base * 1e3
    );
    for (name, fx) in catalog() {
        if !only.is_empty() && !only.split(',').any(|word| name.contains(word)) {
            continue;
        }
        let at = median((0..runs).map(|_| run(Some(&fx), frames)).collect());
        let backdrop = match fx.source().contains("zgx_backdrop") {
            true => "yes",
            false => "",
        };
        println!(
            "| {name} | 960000 | {backdrop} | {:.2} |",
            (at - base) * 1e3 / STACKED as f64
        );
    }
}
