//! Layer shaders with inputs (`FillShader::with_inputs`): passes drawn before
//! each frame, state between frames, pictures, app-drawn views and tagged
//! nodes' boxes, all read by name from the shader.
use std::sync::Arc;
use zgx::decoration::{
    FillShader, PassFormat, PassSize, ShaderInputs, ShaderPass, ShaderUniforms, geometry_tag,
};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 64;
fn pass(name: &str, source: &str) -> ShaderPass {
    ShaderPass {
        name: name.into(),
        source: source.into(),
        size: PassSize::Pixels(4, 4),
        format: PassFormat::Rgba16Float,
        feedback: false,
        steps: 1,
        once: false,
    }
}
fn fixed(size: f32) -> Style {
    Style {
        width: Some(size),
        height: Some(size),
        ..Default::default()
    }
}
struct Fixture {
    gpu: GpuRenderer,
    scene: Scene,
    node: NodeId,
}
fn fixture(shader: Arc<FillShader>) -> Fixture {
    let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(SIZE as f32),
    );
    scene.set_layer_shader(node, Some(shader));
    gpu.debug_wait_for_shaders();
    Fixture { gpu, scene, node }
}
impl Fixture {
    /// A frame at `time`: its pixel in the middle.
    fn frame(&mut self, time: f32) -> [u8; 4] {
        let params = [[0.; 4]; 4];
        (self.scene).set_layer_uniforms(self.node, ShaderUniforms { time, params });
        let report = self.scene.flush();
        self.gpu.render(&self.scene, &report.damage).unwrap();
        let pixels = self.gpu.readback().unwrap();
        let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        pixels[at..at + 4].try_into().unwrap()
    }
}
const SHOW_COUNT: &str = "
fn layer(input: LayerInput) -> vec4<f32> {
    return vec4(zgx_count(vec2(0.5)).r / 255.0, 0.0, 0.0, 1.0);
}";

#[test]
fn a_feedback_pass_keeps_state_from_frame_to_frame() {
    let _gpu = lock();
    let shader = FillShader::with_inputs(
        SHOW_COUNT,
        None,
        ShaderInputs {
            passes: vec![ShaderPass {
                feedback: true,
                ..pass(
                    "count",
                    "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(zgx_count(input.uv).r + 1.0, 0.0, 0.0, 1.0);
                    }",
                )
            }],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    let reds: Vec<u8> = (0..4).map(|n| f.frame(n as f32)[0]).collect();
    assert_eq!(reds, [1, 2, 3, 4], "a frame adds one to the last");
}

#[test]
fn steps_run_a_pass_that_many_times_a_frame() {
    let _gpu = lock();
    let shader = FillShader::with_inputs(
        SHOW_COUNT,
        None,
        ShaderInputs {
            passes: vec![ShaderPass {
                feedback: true,
                steps: 5,
                ..pass(
                    "count",
                    "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(zgx_count(input.uv).r + 1.0, f32(input.step), 0.0, 1.0);
                    }",
                )
            }],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    assert_eq!(f.frame(0.)[0], 5);
    assert_eq!(f.frame(1.)[0], 10);
}

#[test]
fn a_once_pass_draws_once() {
    let _gpu = lock();
    // What it draws depends on time: were it drawn again, red would move.
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(zgx_still(vec2(0.5)).r, 0.0, 0.0, 1.0);
        }",
        None,
        ShaderInputs {
            passes: vec![ShaderPass {
                once: true,
                ..pass(
                    "still",
                    "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(input.time * 0.1, 0.0, 0.0, 1.0);
                    }",
                )
            }],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    let first = f.frame(5.);
    assert!(first[0].abs_diff(128) <= 1, "{first:?}");
    assert_eq!(f.frame(9.), first);
}

#[test]
fn a_later_pass_reads_an_earlier_ones_result_from_this_frame() {
    let _gpu = lock();
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(zgx_second(vec2(0.5)).rg, 0.0, 1.0);
        }",
        None,
        ShaderInputs {
            passes: vec![
                pass(
                    "first",
                    "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(input.time * 0.1, 0.0, 0.0, 1.0);
                    }",
                ),
                pass(
                    "second",
                    "fn run(input: PassInput) -> vec4<f32> {
                        return vec4(zgx_first(input.uv).r, 0.25, 0.0, 1.0);
                    }",
                ),
            ],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    let shown = f.frame(5.);
    assert!(
        shown[0].abs_diff(128) <= 1 && shown[1].abs_diff(64) <= 1,
        "{shown:?}"
    );
}

#[test]
fn a_pass_past_one_keeps_float_values() {
    let _gpu = lock();
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(zgx_big(vec2(0.5)).r / 100.0, -zgx_big(vec2(0.5)).g / 100.0, 0.0, 1.0);
        }",
        None,
        ShaderInputs {
            passes: vec![pass(
                "big",
                "fn run(input: PassInput) -> vec4<f32> {
                    return vec4(50.0, -25.0, 0.0, 1.0);
                }",
            )],
            ..Default::default()
        },
    );
    let shown = fixture(shader).frame(0.);
    assert!(
        shown[0].abs_diff(128) <= 1 && shown[1].abs_diff(64) <= 1,
        "{shown:?}"
    );
}

#[test]
fn a_picture_is_sampled_by_name() {
    let _gpu = lock();
    let image = Arc::new(zgx::image::ImageData::new(1, 1, vec![10, 200, 30, 255]).unwrap());
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(zgx_photo(vec2(0.5)).rgb, 1.0);
        }",
        None,
        ShaderInputs {
            images: vec![("photo".into(), image)],
            ..Default::default()
        },
    );
    let shown = fixture(shader).frame(0.);
    assert!(
        shown[1].abs_diff(200) <= 1 && shown[0].abs_diff(10) <= 1,
        "{shown:?}"
    );
}

#[test]
fn tagged_boxes_are_read_where_they_are_this_frame() {
    let _gpu = lock();
    let tag = geometry_tag("cards");
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            let inside = zgx_rect_count() == 1u && zgx_rect_distance(zgx_window(input.p), 0u) < 0.0;
            return vec4(select(0.0, 1.0, inside), f32(zgx_rect_count()) * 0.25, 0.0, 1.0);
        }",
        None,
        ShaderInputs {
            geometry: Some(tag),
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    assert_eq!(f.frame(0.)[0], 0, "no box tagged yet");
    let root = f.scene.root();
    let card = f
        .scene
        .append(root, NodeKind::Rect(Color(0, 0, 0, 0)), fixed(8.));
    f.scene.set_geometry_tag(card, tag);
    f.scene.set_transform(card, Transform { x: 0., y: 0. });
    assert_eq!(f.frame(1.)[0], 0, "its box is at the corner");
    f.scene.set_transform(card, Transform { x: 28., y: 28. });
    assert_eq!(f.frame(2.)[0], 255, "moved under the middle");
    f.scene.remove(card);
    assert_eq!(f.frame(3.)[1], 0, "gone, it isn't read");
}

#[test]
fn an_apps_view_is_drawn_with_its_device_and_sampled_by_name() {
    let _gpu = lock();
    struct Clearing;
    impl zgx_gpu::viewport::Viewport for Clearing {
        fn render(&mut self, frame: &mut zgx_gpu::viewport::ViewportFrame<'_>) {
            frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame.target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.,
                                g: 1.,
                                b: 0.,
                                a: 1.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
        }
    }
    let view = zgx_gpu::viewport::view(Clearing);
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> {
            return vec4(zgx_app(vec2(0.5)).rgb, 1.0);
        }",
        None,
        ShaderInputs {
            views: vec![("app".into(), view.id())],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    // Its first frame is drawn after the passes, ahead of the frame showing it.
    assert_eq!(f.frame(0.)[1], 255);
    drop(view);
}

#[test]
fn a_shader_with_passes_is_checked_by_name() {
    let good = ShaderInputs {
        passes: vec![pass(
            "a",
            "fn run(input: PassInput) -> vec4<f32> { return zgx_a(input.uv); }",
        )],
        ..Default::default()
    };
    let shader = "fn layer(input: LayerInput) -> vec4<f32> { return zgx_a(vec2(0.0)); }";
    assert_eq!(zgx_gpu::check_layer_inputs(shader, &good), Ok(()));
    let bad = ShaderInputs {
        passes: vec![pass(
            "a",
            "fn run(input: PassInput) -> vec4<f32> { return nope; }",
        )],
        ..Default::default()
    };
    let error = zgx_gpu::check_layer_inputs(shader, &bad).unwrap_err();
    assert!(error.contains("pass `a`"), "{error}");
    let badly_named = ShaderInputs {
        passes: vec![pass(
            "a b",
            "fn run(input: PassInput) -> vec4<f32> { return vec4(0.0); }",
        )],
        ..Default::default()
    };
    assert!(zgx_gpu::check_layer_inputs(shader, &badly_named).is_err());
}

#[test]
fn a_shader_without_inputs_makes_no_chain() {
    let _gpu = lock();
    let shader = FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> { return vec4(1.0, 0.0, 0.0, 1.0); }",
        None,
    );
    let mut f = fixture(shader);
    f.frame(0.);
    assert_eq!(f.gpu.effect_chains(), 0);
}

#[test]
fn an_effect_gone_frees_its_textures() {
    let _gpu = lock();
    let shader = FillShader::with_inputs(
        SHOW_COUNT,
        None,
        ShaderInputs {
            passes: vec![pass(
                "count",
                "fn run(input: PassInput) -> vec4<f32> { return vec4(1.0); }",
            )],
            ..Default::default()
        },
    );
    let mut f = fixture(shader);
    f.frame(0.);
    assert_eq!(f.gpu.effect_chains(), 1);
    f.scene.remove(f.node);
    let report = f.scene.flush();
    f.gpu.render(&f.scene, &report.damage).unwrap();
    f.gpu.trim();
    assert_eq!(f.gpu.effect_chains(), 0, "dropped with its shader");
}

#[test]
fn a_scene_knows_whether_any_layer_shader_has_inputs() {
    // A renderer prepares inputs only in frames that have any: the scene
    // says so as shaders come and go, and as their nodes do.
    let mut scene = Scene::new(SIZE as f32, SIZE as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(SIZE as f32),
    );
    scene.set_layer_shader(node, Some(FillShader::new(SHOW_COUNT, None)));
    assert!(!scene.has_layer_inputs(), "a plain layer shader has none");
    let with = FillShader::with_inputs(
        SHOW_COUNT,
        None,
        ShaderInputs {
            passes: vec![pass(
                "count",
                "fn run(input: PassInput) -> vec4<f32> { return vec4(1.0); }",
            )],
            ..Default::default()
        },
    );
    scene.set_layer_shader(node, Some(with.clone()));
    assert!(scene.has_layer_inputs());
    scene.set_layer_shader(node, None);
    assert!(!scene.has_layer_inputs(), "its shader taken away");
    scene.set_layer_shader(node, Some(with));
    assert!(scene.has_layer_inputs());
    scene.remove(node);
    assert!(!scene.has_layer_inputs(), "its node removed");
}

#[test]
fn a_shader_declared_as_a_script_declares_it_draws_through_its_passes() {
    let _gpu = lock();
    // `ui.layer_shader` declares its shader for every renderer to start
    // before it draws: one with inputs starts with their bindings, so its
    // layer draws through it, not as painted.
    let shader = FillShader::with_inputs(
        "fn layer(input: LayerInput) -> vec4<f32> { return zgx_red(vec2(0.5)); }",
        None,
        ShaderInputs {
            passes: vec![pass(
                "red",
                "fn run(input: PassInput) -> vec4<f32> { return vec4(1.0, 0.0, 0.0, 1.0); }",
            )],
            ..Default::default()
        },
    );
    zgx::decoration::declare_layer_shader(&shader);
    let mut f = fixture(shader);
    assert_eq!(f.frame(0.), [255, 0, 0, 255]);
}
