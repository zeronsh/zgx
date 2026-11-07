//! A page over a wallpaper that follows the pointer: what a frame costs when
//! only layers move. Three pictures, each its own layer, spring after a
//! pointer moving at 120 events a second, under a layer shader over the whole
//! wallpaper (grain and a vignette, its time moving too); over them a still
//! page of text and boxes. Each frame moves the pictures and nothing else.
//! `SCENE=still` moves nothing, for comparison. `FRAMES` (default 600).
//! `READBACK=1` waits for each frame by reading it back, and prints the mean
//! time a frame took, CPU and GPU together.
//! (Here for zgx-3d's wgpu with its no-op backend, as fx_bench.)
//! Run: cargo run --release -p zgx-3d --example parallax_bench
use std::sync::Arc;
use zgx::decoration::{FillShader, ShaderUniforms};
use zgx::image::ImageData;
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const W: f32 = 1200.;
const H: f32 = 800.;
const GRAIN: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let q = input.p / input.size - 0.5;
    let vignette = 1.0 - dot(q, q) * 1.2;
    let n = fract(sin(dot(floor(input.p) + input.time, vec2(12.9898, 78.233))) * 43758.5453);
    return vec4(input.color.rgb * vignette + (n - 0.5) * 0.04, input.color.a);
}";

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
/// A picture of `w` by `h` pixels: soft bands of colour, as a photo's.
fn picture(w: u32, h: u32, seed: u32) -> Arc<ImageData> {
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / w as f32;
            let v = y as f32 / h as f32;
            let k = seed as f32;
            pixels.extend_from_slice(&[
                (120. + 100. * (u * 3. + k).sin()) as u8,
                (110. + 90. * (v * 4. + k * 2.).cos()) as u8,
                (140. + 80. * ((u + v) * 5. + k).sin()) as u8,
                255,
            ]);
        }
    }
    Arc::new(ImageData::new(w, h, pixels).expect("its pixels"))
}

fn main() {
    let scene_name = std::env::var("SCENE").unwrap_or_else(|_| "parallax".into());
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|frames| frames.parse().ok())
        .unwrap_or(600);
    let readback = std::env::var_os("READBACK").is_some();
    let scale = 2.;
    let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).expect("a renderer");
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(W, H);
    gpu.install_text(&mut scene);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    // The wallpaper: pictures larger than the window, each a layer of its
    // own (an opacity under one makes it one), under a layer shader.
    let wallpaper = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(W, H));
    let mut layers = Vec::new();
    for (index, (w, h)) in [(1500., 1000.), (900., 600.), (500., 400.)]
        .into_iter()
        .enumerate()
    {
        let image = picture((w / 2.) as u32, (h / 2.) as u32, index as u32);
        let node = scene.append(wallpaper, NodeKind::Image(image), fixed(w, h));
        scene.set_effects(
            node,
            Effects {
                opacity: 0.98,
                ..Default::default()
            },
        );
        layers.push((node, w, h, 0.02 + 0.03 * index as f32));
    }
    scene.set_layer_shader(wallpaper, Some(FillShader::new(GRAIN, None)));
    // The page: a column of rows of text and boxes, still.
    let page = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(W, H));
    for row in 0..24 {
        let y = 40. + row as f32 * 30.;
        let card = scene.append(
            page,
            NodeKind::Quad(QuadStyle {
                fill: Color(20, 20, 26, 200),
                radius: 6.,
                ..Default::default()
            }),
            fixed(700., 26.),
        );
        scene.set_transform(card, Transform { x: 250., y });
        let text = scene.append(
            page,
            NodeKind::Text {
                text: format!("Row {row}: the quick brown fox jumps over the lazy dog").into(),
                color: Color(235, 235, 240, 255),
                font_size: 14.,
            },
            Style::default(),
        );
        scene.set_transform(text, Transform { x: 262., y: y + 4. });
    }
    let moving = scene_name != "still";
    // Springs after the pointer, stepped at the display's rate.
    let mut springs: Vec<([f32; 2], [f32; 2])> = vec![([0.; 2], [0.; 2]); layers.len()];
    let started = std::time::Instant::now();
    // `DUMP=dir`: frames 1, 60 and the last read back into files there, to
    // compare two builds pixel for pixel.
    let dump = std::env::var("DUMP").ok();
    let mut totals = [0usize; 5];
    for frame in 0..frames {
        let t = frame as f32 / 120.;
        let pointer = [
            W * (0.5 + 0.4 * (t * 0.9).sin()),
            H * (0.5 + 0.35 * (t * 1.3).cos()),
        ];
        if moving {
            for ((node, w, h, depth), (position, velocity)) in layers.iter().zip(&mut springs) {
                for axis in 0..2 {
                    let target = (pointer[axis] - [W, H][axis] / 2.) * -depth;
                    let force = 170. * (target - position[axis]) - 26. * velocity[axis];
                    velocity[axis] += force / 120.;
                    position[axis] += velocity[axis] / 120.;
                }
                scene.set_transform(
                    *node,
                    Transform {
                        x: (W - w) / 2. + position[0],
                        y: (H - h) / 2. + position[1],
                    },
                );
            }
            scene.set_layer_uniforms(
                wallpaper,
                ShaderUniforms {
                    time: t,
                    params: [[0.; 4]; 4],
                },
            );
        }
        let report = scene.flush();
        let stats = gpu.render(&scene, &report.damage).expect("a frame");
        if readback {
            gpu.readback().expect("its pixels");
        }
        if let Some(dir) = &dump
            && (frame == 1 || frame == 60 || frame + 1 == frames)
        {
            let pixels = gpu.readback().expect("its pixels");
            std::fs::write(format!("{dir}/{scene_name}-{frame}.rgba"), pixels).expect("written");
        }
        for (total, value) in totals.iter_mut().zip([
            stats.instances,
            stats.layer_repaints,
            stats.layer_cache_hits,
            stats.patched_items,
            stats.draw_calls,
        ]) {
            *total += value;
        }
        if frame + 1 == frames {
            let [instances, repaints, hits, patched, draws] = totals;
            println!(
                "{scene_name}: {frames} frames; in all {instances} instances, {draws} draws, {repaints} layer repaints, {hits} cache hits, {patched} items patched"
            );
        }
    }
    if readback {
        println!(
            "{:.3} ms a frame, CPU and GPU, read back",
            started.elapsed().as_secs_f64() * 1e3 / frames as f64
        );
    }
}
