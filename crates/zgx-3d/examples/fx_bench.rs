//! What an effect costs a frame's CPU, through the whole renderer: rows of
//! boxes, one of them changing each frame, under `SCENE`: `none`,
//! as they are; `layer`, a card over them with an animated layer shader
//! painting past its box; `reader`, one reading what's behind it (damage
//! reaching what it reads, the backdrop's copy, a pass of its own); `passes`,
//! one with a feedback pass and tagged boxes as inputs. The first
//! is what a window with no effect pays, the rest show zero can be told from
//! it. The gate runs them on wgpu's no-op backend (`ZGX_GPU_NOOP`).
//! (Here for zgx-3d's wgpu with its no-op backend, not to build that into
//! zgx-gpu's own tests.) Run: cargo run --release -p zgx-3d --example fx_bench
use std::sync::Arc;
use zgx::decoration::{FillShader, PassFormat, PassSize, ShaderInputs, ShaderPass, ShaderUniforms};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const GLOW: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let outside = max(max(-input.p, input.p - input.size), vec2(0.0));
    let d = length(outside);
    let glow = exp(-d * 0.1) * (0.5 + 0.5 * sin(input.time));
    return vec4(mix(vec3(0.2, 0.7, 1.0), input.color.rgb, input.color.a), max(input.color.a, glow));
}";
const HAZE: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    let shift = vec2(sin(input.p.y * 0.2 + input.time) * input.params[0].x, 0.0);
    let behind = zgx_backdrop(input.p + shift);
    return vec4(mix(behind.rgb, input.color.rgb, input.color.a), max(behind.a, input.color.a));
}";

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

fn main() {
    run(std::env::var("SCENE").unwrap_or_else(|_| "reader".into()));
}
/// The frames of scene `scene_name`.
pub fn run(scene_name: String) {
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|frames| frames.parse().ok())
        .unwrap_or(600);
    let mut gpu = GpuRenderer::new(800, 600).expect("a renderer");
    let mut scene = Scene::new(800., 600.);
    let fonts = gpu.text_system();
    scene.set_text_measurer(move |text: &str, size: f32, width: Option<f32>| {
        zgx_gpu::measure_text(&mut fonts.borrow_mut(), text, size, width)
    });
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let mut rows = Vec::new();
    for row in 0..40 {
        let y = row as f32 * 15.;
        let id = scene.append(
            root,
            NodeKind::Rect(Color(40 + row, 60, 90, 255)),
            fixed(800., 14.),
        );
        scene.set_transform(id, Transform { x: 0., y });
        rows.push(id);
    }
    let card = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        fixed(300., 200.),
    );
    scene.set_transform(card, Transform { x: 250., y: 200. });
    scene.append(
        card,
        NodeKind::Rect(Color(255, 255, 255, 30)),
        fixed(280., 180.),
    );
    let shader: Option<Arc<FillShader>> = match scene_name.as_str() {
        "layer" => Some(FillShader::new(GLOW, None)),
        "passes" => Some(FillShader::with_inputs(
            "fn layer(input: LayerInput) -> vec4<f32> {
                return vec4(zgx_state(input.p / input.size).rgb, f32(zgx_rect_count()) * 0.1);
            }",
            None,
            ShaderInputs {
                passes: vec![ShaderPass {
                    name: "state".into(),
                    source: "fn run(input: PassInput) -> vec4<f32> {
                        return zgx_state(input.uv) * 0.9 + vec4(input.dt);
                    }"
                    .into(),
                    size: PassSize::Pixels(64, 64),
                    format: PassFormat::Rgba16Float,
                    feedback: true,
                    steps: 4,
                    once: false,
                }],
                geometry: Some(zgx::decoration::geometry_tag("rows")),
                ..Default::default()
            },
        )),
        "reader" => Some(FillShader::new(HAZE, None)),
        _ => None,
    };
    let (bleed, reach) = match scene_name.as_str() {
        "layer" => (24., 0.),
        "reader" => (0., 12.),
        _ => (0., 0.),
    };
    if scene_name == "passes" {
        for row in rows.iter().take(8) {
            scene.set_geometry_tag(*row, zgx::decoration::geometry_tag("rows"));
        }
    }
    scene.set_layer_shader(card, shader.clone());
    scene.set_layer_extent(card, bleed, reach);
    for frame in 0..frames {
        let row = rows[frame % rows.len()];
        let shade = (frame % 200) as u8;
        scene.set_kind(row, NodeKind::Rect(Color(40, shade, 90, 255)));
        if shader.is_some() {
            scene.set_layer_uniforms(
                card,
                ShaderUniforms {
                    time: frame as f32 / 60.,
                    params: [[6., 0., 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
                },
            );
        }
        let report = scene.flush();
        gpu.render(&scene, &report.damage).expect("a frame");
    }
    println!(
        "{scene_name}: {frames} frames, {:?}",
        gpu.debug_cache_stats().layer_textures
    );
}
