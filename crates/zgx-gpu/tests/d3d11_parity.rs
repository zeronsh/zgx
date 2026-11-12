//! Direct3D 11 (`ZGX_D3D11=1`, the `d3d11` feature) draws what Direct3D 12
//! does: cards whose bars and values change each frame (kept frames
//! patched), text through the glyph atlas, rounded borders, a list scrolled
//! inside a rounded clip (scroll copies, clip regions), and heavy's
//! continuous geometry (bars and a progress bar with fractional edges),
//! isolated layers (translucent, nested, and turning inside), and images
//! (a picture with straight alpha and a gradient's ramp), and canvas
//! polylines (a moving sparkline, a translucent stroke over itself), and
//! a truncated text's blurred shadow reaching past its box, and a counter
//! filled with a gradient (source-in and behind blends), and every
//! ready-made shader fill (`zgx_fx`'s, its fragment stage compiled for each
//! as an app's), and layer effects (content blurs, narrow to wide, and
//! masks, alone, sliding and under a blur, and drop shadows), and images
//! drawn dithered and as characters, and layers in every blend mode, at
//! 1×, 1.5× and 2×, byte for byte: both compile the same HLSL with FXC
//! (Direct3D 12 pinned to it, docs/windows.md). `ZGX_D3D11` is read once a
//! process, so the test runs itself again with it and compares that run's
//! frames with its own.
//!
//! One allowance, on a hardware adapter alone (`HARDWARE_ONE_OFF`): an app's
//! shaders reach the two APIs as different DXBC (Direct3D 12's through its
//! sampler heap, `dx12_fill`, as ps_5_1; Direct3D 11's with its sampler
//! bound, as ps_5_0), which FXC lays out apart. WARP draws both alike to the
//! byte, but a driver's own compilers may round a value on a half-step
//! apart: on an RTX 4090, the very DXBC Direct3D 12 draws (its dump byte for
//! byte WARP's) gives fire's green 112 where WARP gives 113, as Direct3D 11
//! does there. So in a frame of app shaders a hardware adapter may differ by
//! one level in a few bytes, each printed; WARP stays exact.
#[cfg(windows)]
use zgx::{compose::prelude::*, reactive::Signal, scene::Color, widgets::Ui};
#[cfg(windows)]
use zgx_gpu::{GpuRenderer, text::FontData};

#[cfg(windows)]
#[path = "support/view_painters.rs"]
mod view_painters;
#[cfg(windows)]
const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
#[cfg(windows)]
const SIZE: (f32, f32) = (400., 300.);
#[cfg(windows)]
const SCALES: [f32; 3] = [1., 1.5, 2.];
#[cfg(windows)]
const FRAMES: u32 = 40;
/// Set in the run drawing with Direct3D 11.
#[cfg(windows)]
const CHILD: &str = "ZGX_D3D11_PARITY_CHILD";

#[cfg(windows)]
fn view(live: Signal<u32>, offset: Signal<f32>) -> View {
    let (spark, progress, turn, wave) = (live.clone(), live.clone(), live.clone(), live.clone());
    let counted = live.clone();
    let card = move |index: u32| {
        let (value, bars) = (live.clone(), live.clone());
        div()
            .absolute()
            .ml(8. + 128. * index as f32)
            .mt(8.)
            .size(120., 110.)
            .rounded(6.)
            .border(1.)
            .border_color(Color(40, 50, 64, 255))
            .bg(Color(22, 28, 38, 255))
            .child(
                text_signal(move || format!("{:.1} ms", 3.7 * (value.get() + index) as f32))
                    .text_size(15.)
                    .absolute()
                    .ml(8.)
                    .mt(6.)
                    .size(100., 20.),
            )
            .children((0..12).map(move |bar| {
                let bars = bars.clone();
                div()
                    .absolute()
                    .ml(8. + 8.5 * bar as f32)
                    .w(5.)
                    .rounded(2.)
                    .bg(Color(94, 177, 255, 255))
                    .reactive_style(move || {
                        let height = 2. + ((bars.get() + bar + index) % 9) as f32 * 7.25;
                        Styles::new().mt(102. - height).h(height)
                    })
            }))
    };
    let list = virtual_list(
        offset,
        22.,
        2,
        || 500,
        |index| index,
        |_, index, _| {
            div()
                .w(376.)
                .h(22.)
                .bg(if index % 2 == 0 {
                    Color(25, 34, 48, 255)
                } else {
                    Color(29, 40, 56, 255)
                })
                .child(
                    text(format!("row {index}"))
                        .text_size(13.)
                        .ml(6.)
                        .mt(3.)
                        .size(200., 16.),
                )
        },
    )
    .absolute()
    .ml(12.)
    .mt(130.)
    .w(376.)
    .h(150.)
    .rounded(10.)
    .overflow_hidden();
    // As heavy's sparklines and progress bars: edges between pixels that move
    // each frame, inside a rounded card.
    let continuous = div()
        .absolute()
        .ml(12.)
        .mt(286.)
        .size(376., 12.)
        .rounded(3.)
        .bg(Color(30, 38, 52, 255))
        .overflow_hidden()
        .children((0..40).map(move |bar| {
            let spark = spark.clone();
            div()
                .absolute()
                .ml(2.3 + 9.1 * bar as f32)
                .w(6.3)
                .bg(Color(126, 231, 135, 255))
                .reactive_style(move || {
                    let height = 1.7 + ((spark.get() * 3 + bar * 7) % 23) as f32 * 0.37;
                    Styles::new().mt(12. - height).h(height)
                })
        }))
        .child(
            div()
                .absolute()
                .mt(0.)
                .h(3.4)
                .bg(Color(255, 196, 64, 255))
                .reactive_style(move || {
                    Styles::new().w(376. * ((progress.get() % 37) as f32 + 0.43) / 37.)
                }),
        );
    // Layers: a translucent isolated subtree over the third card, a nested
    // one inside it, and a square turning in that, each frame.
    let layered = div()
        .absolute()
        .ml(250.)
        .mt(40.)
        .size(110., 90.)
        .isolated(true)
        .opacity(0.7)
        .rounded(8.)
        .bg(Color(120, 72, 200, 255))
        .child(
            text("layer")
                .text_size(13.)
                .absolute()
                .ml(8.)
                .mt(4.)
                .size(80., 16.),
        )
        .child(
            div()
                .absolute()
                .ml(30.)
                .mt(26.)
                .size(56., 56.)
                .isolated(true)
                .opacity(0.8)
                .rounded(28.)
                .bg(Color(255, 140, 90, 255))
                .child(
                    div()
                        .absolute()
                        .ml(14.)
                        .mt(14.)
                        .size(28., 28.)
                        .bg(Color(240, 240, 250, 255))
                        .reactive_style(move || Styles::new().rotate(turn.get() as f32 * 0.13)),
                ),
        );
    // Images: a picture with straight alpha, scaled between pixels, and a
    // gradient (its ramp an image too), over the second card.
    let pixels: Vec<u8> = (0..16 * 12)
        .flat_map(|i: u32| {
            let (x, y) = (i % 16, i / 16);
            [
                (x * 16) as u8,
                (y * 21) as u8,
                180,
                if (x + y) % 3 == 0 { 96 } else { 255 },
            ]
        })
        .collect();
    let picture = std::sync::Arc::new(zgx::image::ImageData::new(16, 12, pixels).unwrap());
    let pictured = div()
        .absolute()
        .ml(140.)
        .mt(36.)
        .size(112., 64.)
        .child(image("picture", picture).absolute().size(53.5, 40.25))
        .child(
            div()
                .absolute()
                .ml(56.)
                .size(56., 64.)
                .rounded(6.)
                .bg_gradient(
                    0.7,
                    vec![
                        zgx::canvas::GradientStop {
                            offset: 0.,
                            color: Color(255, 90, 120, 255),
                        },
                        zgx::canvas::GradientStop {
                            offset: 1.,
                            color: Color(40, 200, 255, 160),
                        },
                    ],
                ),
        );
    // Polylines: a sparkline moving each frame, mitred, and a translucent
    // round stroke over itself, on a canvas over the first card.
    let lines = canvas(move |_| {
        use zgx::canvas::{Canvas, Path, Stroke};
        let line = |points: &[(f32, f32)]| {
            let mut path = Path::builder();
            path.move_to(points[0].0, points[0].1);
            for &(x, y) in &points[1..] {
                path.line_to(x, y);
            }
            path.build().unwrap()
        };
        let phase = wave.get() as f32 * 0.31;
        let spark: Vec<(f32, f32)> = (0..40)
            .map(|k| {
                let t = k as f32;
                (2.3 + t * 2.4, 14. + 7. * (t * 0.7 + phase).sin())
            })
            .collect();
        let mut canvas = Canvas::new();
        canvas.stroke(
            line(&spark),
            Color(63, 185, 80, 255),
            Stroke {
                width: 1.5,
                ..Default::default()
            },
        );
        canvas.stroke(
            line(&[(4., 40.), (30., 22.), (52., 44.), (90., 26.)]),
            Color(255, 255, 255, 140),
            Stroke {
                width: 5.,
                ..Default::default()
            },
        );
        canvas
    })
    .absolute()
    .ml(12.)
    .mt(36.)
    .size(100., 50.);
    // A truncated text whose blurred shadow reaches past its box: its
    // glyphs clipped to the box, its shadow not.
    let shadowed = text("HHHH HHHH HHHH HHHH")
        .truncate()
        .size(130., 20.)
        .text_size(14.)
        .text_color(Color(229, 237, 247, 255))
        .text_shadow(0., 12., 3., Color(230, 60, 60, 255))
        .absolute()
        .ml(260.)
        .mt(250.);
    // A counter filled with a gradient, with a shadow: its own layer's
    // glyphs, the gradient source-in and the shadow behind them.
    let filled = text_signal(move || format!("{} frames", counted.get()))
        .size(110., 24.)
        .text_size(18.)
        .text_shadow(1., 2., 0., Color(20, 20, 20, 255))
        .text_gradient(zgx::decoration::Gradient::Linear {
            angle: 0.3,
            stops: vec![
                zgx::canvas::GradientStop {
                    offset: 0.,
                    color: Color(255, 120, 40, 255),
                },
                zgx::canvas::GradientStop {
                    offset: 1.,
                    color: Color(80, 140, 255, 255),
                },
            ]
            .into(),
        })
        .absolute()
        .ml(140.)
        .mt(270.);
    div()
        .w(SIZE.0)
        .h(SIZE.1)
        .bg(Color(16, 20, 28, 255))
        .text_color(Color(229, 237, 247, 255))
        .children((0..3).map(card))
        .child(list)
        .child(continuous)
        .child(layered)
        .child(pictured)
        .child(lines)
        .child(shadowed)
        .child(filled)
}

/// Every ready-made shader fill (`zgx_fx`, with its presets) in a box of
/// its own with rounded corners, at a few moments, as this process draws
/// them at `scale`: each fill's module through FXC (in a process of its own,
/// `fxc_child`), as an app's would be.
#[cfg(windows)]
fn fill_frames(scale: f32) -> Vec<Vec<u8>> {
    use zgx::scene::*;
    let fills: Vec<_> = zgx_fx::catalog()
        .into_iter()
        .filter(|(_, fx)| fx.kind() == zgx_fx::FxKind::Fill)
        .collect();
    assert!(fills.len() >= 20, "{} fills", fills.len());
    // And a radial gradient's: zgx's own fill, compiled as an app's on
    // Direct3D 11 (in the process, as a built-in, on Direct3D 12).
    let names = fill_names();
    let (cell, columns) = (FILL_CELL, FILL_COLUMNS);
    let size = (
        cell.0 * columns as f32,
        cell.1 * names.len().div_ceil(columns) as f32,
    );
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let now = std::time::Instant::now();
    [0.3, 1.7, 4.1]
        .into_iter()
        .map(|time| {
            let mut scene = Scene::new(size.0, size.1);
            let root = scene.root();
            scene.set_kind(root, NodeKind::Container(Layout::Overlay));
            let radial = zgx::decoration::radial_shader();
            let shaders = fills
                .iter()
                .map(|(_, fx)| (fx.shader(), Some(fx)))
                .chain([(radial, None)]);
            for (index, (shader, fx)) in shaders.enumerate() {
                let quad = QuadStyle {
                    radius: 8.,
                    fill: Color(0, 0, 0, 255),
                    decoration: Some(std::sync::Arc::new(zgx::decoration::Decoration {
                        background: Some(zgx::decoration::Background::Shader(shader)),
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                let id = scene.append(
                    root,
                    NodeKind::Panel {
                        layout: Layout::Overlay,
                        quad,
                    },
                    Style {
                        width: Some(cell.0 - 6.),
                        height: Some(cell.1 - 6.),
                        ..Default::default()
                    },
                );
                scene.set_transform(
                    id,
                    Transform {
                        x: (index % columns) as f32 * cell.0 + 3.,
                        y: (index / columns) as f32 * cell.1 + 3.,
                    },
                );
                match fx {
                    Some(fx) => fx.apply(&mut scene, id, time, false, now),
                    None => scene.set_shader_uniforms(
                        id,
                        zgx::decoration::radial_uniforms(
                            (0.3, 0.4),
                            0.9,
                            [
                                zgx::canvas::GradientStop {
                                    offset: 0.,
                                    color: Color(255, 200, 80, 255),
                                },
                                zgx::canvas::GradientStop {
                                    offset: 1.,
                                    color: Color(40, 90, 220, 255),
                                },
                            ]
                            .into(),
                        ),
                    ),
                }
            }
            let report = scene.flush();
            gpu.render(&scene, &report.damage).unwrap();
            let pixels = gpu.readback().unwrap();
            // Each box shows its fill, not the black under it (a fill that
            // failed to compile here and in Direct3D 12 alike would match).
            let width = (size.0 * scale) as usize;
            for (index, name) in names.iter().enumerate() {
                let (x0, y0) = (
                    ((index % columns) as f32 * cell.0 + 8.) * scale,
                    ((index / columns) as f32 * cell.1 + 8.) * scale,
                );
                let (w, h) = ((cell.0 - 16.) * scale, (cell.1 - 16.) * scale);
                let lit = (y0 as usize..(y0 + h) as usize)
                    .flat_map(|y| (x0 as usize..(x0 + w) as usize).map(move |x| (x, y)))
                    .filter(|&(x, y)| pixels[(y * width + x) * 4..][..3] != [0, 0, 0])
                    .count();
                assert!(
                    lit as f32 > 0.1 * w * h,
                    "{name} at {time}: {lit} pixels drawn of {}",
                    w * h
                );
            }
            pixels
        })
        .collect()
}

/// Every ready-made layer shader reading neither the backdrop nor a blur
/// (`zgx_fx`'s, with presets, one-shots alive), and zgx's colour filter
/// (`hue_rotate`), each over a box holding a bar on striped content, at a
/// few moments, as this process draws them at `scale`; the plain scene
/// first. Each must change its cell at one of the moments: drawn as
/// painted in both processes (a shader that failed to compile in both),
/// they would match.
#[cfg(windows)]
fn layer_frames(scale: f32) -> Vec<Vec<u8>> {
    shader_frames(scale, false)
}

/// As `layer_frames`, of every ready-made layer shader reading the backdrop
/// but not blurring it (glass, `zgx_fx`'s), and a glass of zgx's own mixing
/// the backdrop with the box. Each cell is damaged at each moment: a frame
/// of several regions, whose backdrop reads may reach past their own.
#[cfg(windows)]
fn glass_frames(scale: f32) -> Vec<Vec<u8>> {
    shader_frames(scale, true)
}

/// The frames `layer_frames` or (`glass`) `glass_frames` draw.
#[cfg(windows)]
fn shader_frames(scale: f32, glass: bool) -> Vec<Vec<u8>> {
    use std::time::{Duration, Instant};
    use zgx::decoration::{ShaderUniforms, color_filter_shader};
    use zgx::scene::*;
    let start = Instant::now();
    let mut layers: Vec<(String, Option<zgx_fx::Fx>)> = zgx_fx::catalog()
        .into_iter()
        .filter(|(_, fx)| {
            fx.kind() == zgx_fx::FxKind::Layer
                && fx.frost() == 0.
                && fx.shader().reads_backdrop() == glass
        })
        .map(|(name, fx)| {
            // A one-shot alive since `start`, seen a moment on.
            let _ = fx.uniforms(0., false, start);
            (name, Some(fx))
        })
        .collect();
    match glass {
        false => {
            assert!(layers.len() >= 10, "{} layer shaders", layers.len());
            layers.push(("hue_rotate".into(), None));
        }
        true => layers.push(("glass mix".into(), None)),
    }
    let mix = zgx::decoration::FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    let behind = zgx_backdrop(input.p + vec2(6. * sin(input.time * 3.), 4.));
    return vec4(mix(behind.rgb, input.color.rgb, 0.35), max(behind.a, input.color.a));
}",
        None,
    );
    let (cell, columns) = (LAYER_CELL, LAYER_COLUMNS);
    let size = (
        cell.0 * columns as f32,
        cell.1 * layers.len().div_ceil(columns) as f32,
    );
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let later = start + Duration::from_millis(250);
    let draw = |gpu: &mut GpuRenderer, time: Option<f32>| {
        let mut scene = Scene::new(size.0, size.1);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Overlay));
        for i in 0..(size.1 / 12.).ceil() as usize {
            let stripe = scene.append(
                root,
                NodeKind::Rect(if i % 2 == 0 {
                    Color(30, 60, 200, 255)
                } else {
                    Color(230, 130, 30, 255)
                }),
                Style {
                    width: Some(size.0),
                    height: Some(12.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                stripe,
                Transform {
                    x: 0.,
                    y: i as f32 * 12.,
                },
            );
        }
        // Glass reads the backdrop at offsets across as well as down
        // (chroma_split's split is across): columns over the stripes, so
        // what's behind changes both ways.
        for j in (1..(size.0 / 12.).ceil() as usize)
            .step_by(2)
            .filter(|_| glass)
        {
            let column = scene.append(
                root,
                NodeKind::Rect(Color(40, 200, 90, 140)),
                Style {
                    width: Some(12.),
                    height: Some(size.1),
                    ..Default::default()
                },
            );
            scene.set_transform(
                column,
                Transform {
                    x: j as f32 * 12.,
                    y: 0.,
                },
            );
        }
        for (index, (_, fx)) in layers.iter().enumerate() {
            let id = scene.append(
                root,
                NodeKind::Panel {
                    layout: Layout::Overlay,
                    quad: QuadStyle {
                        radius: 6.,
                        fill: Color(20, 24, 32, 200),
                        // The colour filter's box takes its shader as its
                        // fill too: one shader of both kinds, which Direct3D
                        // 11 compiles from two modules. As a fill it doesn't
                        // validate (no `fill`) and the box draws its colour;
                        // as the layer's shader it must still draw.
                        decoration: (fx.is_none() && !glass).then(|| {
                            std::sync::Arc::new(zgx::decoration::Decoration {
                                background: Some(zgx::decoration::Background::Shader(
                                    color_filter_shader(),
                                )),
                                ..Default::default()
                            })
                        }),
                        ..Default::default()
                    },
                },
                Style {
                    width: Some(48.),
                    height: Some(32.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                id,
                Transform {
                    x: (index % columns) as f32 * cell.0 + 24.,
                    y: (index / columns) as f32 * cell.1 + 20.,
                },
            );
            let bar = scene.append(
                id,
                NodeKind::Rect(Color(250, 250, 250, 255)),
                Style {
                    width: Some(24.),
                    height: Some(8.),
                    ..Default::default()
                },
            );
            scene.set_transform(bar, Transform { x: 12., y: 12. });
            match (time, fx) {
                (None, _) => {}
                (Some(time), Some(fx)) => fx.apply(&mut scene, id, time, false, later),
                (Some(time), None) if glass => {
                    scene.set_layer_shader(id, Some(mix.clone()));
                    scene.set_layer_uniforms(
                        id,
                        ShaderUniforms {
                            time,
                            params: [[0.; 4]; 4],
                        },
                    );
                }
                (Some(_), None) => {
                    let matrix = hue_rotate_matrix(120.);
                    scene.set_layer_shader(id, Some(color_filter_shader()));
                    scene.set_layer_uniforms(
                        id,
                        ShaderUniforms {
                            time: 0.,
                            params: [matrix[0], matrix[1], matrix[2], [0.; 4]],
                        },
                    );
                }
            }
        }
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        gpu.readback().unwrap()
    };
    let plain = draw(&mut gpu, None);
    let mut all = vec![plain.clone()];
    all.extend([0.1, 0.45, 0.8, 1.31].map(|time| draw(&mut gpu, Some(time))));
    let width = (size.0 * scale) as usize;
    for (index, (name, _)) in layers.iter().enumerate() {
        let (x0, y0) = (
            ((index % columns) as f32 * cell.0 * scale) as usize,
            ((index / columns) as f32 * cell.1 * scale) as usize,
        );
        let (w, h) = ((cell.0 * scale) as usize, (cell.1 * scale) as usize);
        let changed = all[1..].iter().any(|frame| {
            (y0..y0 + h).any(|y| {
                (x0..x0 + w)
                    .any(|x| frame[(y * width + x) * 4..][..4] != plain[(y * width + x) * 4..][..4])
            })
        });
        assert!(changed, "{name}: draws as painted at every moment");
    }
    all
}

/// Each frame's pixels, RGBA, as this process draws them at `scale`: the
/// suite's view's, then the fills', the layer shaders' and the layer
/// effects'.
#[cfg(windows)]
fn frames(scale: f32) -> Vec<Vec<u8>> {
    let mut all = suite_frames(scale);
    all.extend(fill_frames(scale));
    all.extend(layer_frames(scale));
    all.extend(effect_frames(scale));
    all.extend(image_effect_frames(scale));
    all.extend(blend_frames(scale));
    all.extend(glass_frames(scale));
    all.extend(cross_region_frames(scale));
    all.extend(view_frames(scale));
    all.extend(view_scene_frames(scale));
    all.extend(blur_frames(scale));
    all.extend(fade_frames(scale));
    all
}

/// The seven GPU-view scenes of `support/view_painters.rs`
/// (`VIEW_SCENE_NAMES`): a lit textured cube with depth, MSAA resolved into
/// the target, a transparent view over UI, a view resized between frames,
/// one animated, two views in a window, and one clipped, rounded, turned,
/// scaled and translucent. Each scene asserts its view drew.
#[cfg(windows)]
fn view_scene_frames(scale: f32) -> Vec<Vec<u8>> {
    let frames = view_painters::view_scene_frames(scale);
    assert_eq!(
        frames.len(),
        view_painters::VIEW_SCENE_NAMES.len(),
        "a GPU is required here"
    );
    frames
}

/// A GPU view (`zgx::gpu_view`) drawn by a painter with wgpu: under
/// Direct3D 11, on the views' own Direct3D 12 device, its texture shared
/// with Direct3D 11 (`view_host`). It clears to a colour and draws a
/// triangle with a pipeline of its own, in a rounded box at 0.8 opacity over
/// a bar, drawn again in a new colour each frame (`GpuView::invalidate`).
/// Each frame must show the view's colour and its triangle: a view left out
/// would show the bar.
#[cfg(windows)]
fn view_frames(scale: f32) -> Vec<Vec<u8>> {
    use std::{cell::Cell, rc::Rc};
    use zgx_gpu::viewport::{Viewport, ViewportFrame};
    struct Painter {
        colour: Rc<Cell<[f64; 3]>>,
        pipeline: Option<wgpu::RenderPipeline>,
    }
    impl Viewport for Painter {
        fn render(&mut self, frame: &mut ViewportFrame<'_>) {
            let (device, format) = (frame.device, frame.format);
            let pipeline = self.pipeline.get_or_insert_with(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("triangle"),
                    source: wgpu::ShaderSource::Wgsl(
                        "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array(vec2(-0.6, -0.6), vec2(0.6, -0.6), vec2(0.0, 0.7));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4(0.95, 0.9, 0.2, 1.0); }"
                            .into(),
                    ),
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("triangle"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            });
            let [r, g, b] = self.colour.get();
            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("view"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame.target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            pass.set_pipeline(pipeline);
            pass.draw(0..3, 0..1);
        }
        fn device_changed(&mut self) {
            self.pipeline = None;
        }
    }
    let size = VIEW_SIZE;
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let colour = Rc::new(Cell::new([0.1, 0.3, 0.8]));
    let view = zgx_gpu::viewport::view(Painter {
        colour: colour.clone(),
        pipeline: None,
    });
    let mut ui = Ui::new(size.0, size.1);
    ui.mount(
        div()
            .size(size.0, size.1)
            .bg(Color(20, 24, 32, 255))
            .child(
                div()
                    .absolute()
                    .ml(10.)
                    .mt(60.)
                    .size(220., 40.)
                    .bg(Color(230, 130, 30, 255)),
            )
            .child(
                gpu_view("view", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .size(160., 120.)
                    .rounded(12.)
                    .opacity(0.8),
            ),
    );
    [[0.1, 0.3, 0.8], [0.8, 0.2, 0.3], [0.2, 0.7, 0.4]]
        .into_iter()
        .map(|next| {
            colour.set(next);
            view.invalidate();
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            let pixels = gpu.readback().unwrap();
            // The view's middle shows its triangle, and its corner the
            // colour it cleared to (over the bar, at 0.8): not the bar.
            let width = (size.0 * scale) as usize;
            let at = |x: f32, y: f32| ((y * scale) as usize * width + (x * scale) as usize) * 4;
            let middle = &pixels[at(120., 80.)..][..4];
            assert!(
                middle[0] > 180 && middle[1] > 170 && middle[2] < 90,
                "the view's triangle isn't drawn: {middle:?} at {scale}x"
            );
            let corner = &pixels[at(60., 70.)..][..4];
            let strongest = |c: [f32; 3]| (0..3).max_by(|&a, &b| c[a].total_cmp(&c[b])).unwrap();
            let drawn = [corner[0], corner[1], corner[2]].map(f32::from);
            assert_eq!(
                strongest(drawn),
                strongest(next.map(|c| c as f32)),
                "the view's clear colour {next:?} isn't drawn: {corner:?} at {scale}x"
            );
            pixels
        })
        .collect()
}

/// A card fading as one across a `fade_edges` band (bands.rs): light paper,
/// rounded and clipping, holding dark art, in a clip whose top 40 points
/// fade, over an opaque ground. Direct3D 12 draws what reaches the band
/// into a texture and fades it once, so the paper never shows through the
/// art as a veil. The card moves down through the band, a frame a step.
#[cfg(windows)]
fn fade_frames(scale: f32) -> Vec<Vec<u8>> {
    use zgx::scene::*;
    let size = FADE_SIZE;
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene.append(
        root,
        NodeKind::Rect(Color(30, 90, 60, 255)),
        Style {
            width: Some(size.0),
            height: Some(size.1),
            ..Default::default()
        },
    );
    let clip = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        Style {
            width: Some(160.),
            height: Some(160.),
            clip: true,
            fade_edges: [40., 0.],
            ..Default::default()
        },
    );
    scene.set_transform(clip, Transform { x: 20., y: 20. });
    let card = scene.append(
        clip,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: Color(240, 238, 232, 255),
                radius: 12.,
                ..Default::default()
            },
        },
        Style {
            width: Some(160.),
            height: Some(80.),
            clip: true,
            ..Default::default()
        },
    );
    let art = zgx::image::ImageData::new(4, 4, [40u8, 40, 80, 255].repeat(16)).unwrap();
    scene.append(
        card,
        NodeKind::Image(std::sync::Arc::new(art)),
        Style {
            width: Some(120.),
            height: Some(60.),
            ..Default::default()
        },
    );
    [0., 13., 27.5, 60.]
        .into_iter()
        .map(|y| {
            scene.set_transform(card, Transform { x: 0., y });
            let report = scene.flush();
            gpu.render(&scene, &report.damage).unwrap();
            gpu.readback().unwrap()
        })
        .collect()
}
#[cfg(windows)]
const VIEW_SIZE: (f32, f32) = (240., 160.);
#[cfg(windows)]
const FADE_SIZE: (f32, f32) = (200., 200.);

/// Backdrop blurs (`Effects::blur_radius`), drawn as Direct3D 12 draws them
/// (blur.wgsl's passes into an atlas, then back over the target): a plain
/// one, one fading at its edges, one saturated and brightened, zgx-fx's
/// frosted lens and fluted glass, and glass reading the backdrop over a blur
/// of its own (`BLUR_NAMES`). A bar moves under them all each frame, so each
/// blurs again. Each cell must change with it, and the plain blur must not
/// be its box opaque (as Direct3D 11 drew one, left out).
#[cfg(windows)]
fn blur_frames(scale: f32) -> Vec<Vec<u8>> {
    use zgx::decoration::{FillShader, ShaderUniforms};
    use zgx::scene::*;
    let (cell, columns) = (BLUR_CELL, 3);
    let size = (cell.0 * columns as f32, cell.1 * 2.);
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let fixed = |width: f32, height: f32| Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    };
    // A checker of stripes and columns, then the bar that moves.
    for i in 0..(size.1 / 12.).ceil() as usize {
        let colour = match i % 2 {
            0 => Color(30, 60, 200, 255),
            _ => Color(230, 130, 30, 255),
        };
        let stripe = scene.append(root, NodeKind::Rect(colour), fixed(size.0, 12.));
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 12.,
            },
        );
    }
    for j in (1..(size.0 / 12.).ceil() as usize).step_by(2) {
        let column = scene.append(
            root,
            NodeKind::Rect(Color(40, 200, 90, 140)),
            fixed(12., size.1),
        );
        scene.set_transform(
            column,
            Transform {
                x: j as f32 * 12.,
                y: 0.,
            },
        );
    }
    let bar = scene.append(
        root,
        NodeKind::Rect(Color(250, 250, 250, 255)),
        fixed(30., size.1),
    );
    let fx = |name: &str| {
        zgx_fx::catalog()
            .into_iter()
            .find(|(fx_name, _)| fx_name == name)
            .unwrap_or_else(|| panic!("no {name} in zgx-fx's catalog"))
            .1
    };
    let mix = FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    let behind = zgx_backdrop(input.p + vec2(3., 2.));
    return vec4(mix(behind.rgb, input.color.rgb, 0.3), max(behind.a, input.color.a));
}",
        None,
    );
    let now = std::time::Instant::now();
    let boxes: Vec<NodeId> = (0..BLUR_NAMES.len())
        .map(|index| {
            let id = scene.append(
                root,
                NodeKind::Panel {
                    layout: Layout::Overlay,
                    quad: QuadStyle {
                        radius: 8.,
                        fill: Color(20, 24, 32, 110),
                        ..Default::default()
                    },
                },
                fixed(cell.0 - 32., cell.1 - 24.),
            );
            scene.set_transform(
                id,
                Transform {
                    x: (index % columns) as f32 * cell.0 + 16.,
                    y: (index / columns) as f32 * cell.1 + 12.,
                },
            );
            let blur = |radius: f32| Effects {
                blur_radius: radius,
                ..Default::default()
            };
            match index {
                0 => scene.set_effects(id, blur(12.)),
                1 => scene.set_effects(
                    id,
                    Effects {
                        edge_fade: 12.,
                        ..blur(8.)
                    },
                ),
                2 => scene.set_effects(
                    id,
                    Effects {
                        saturate: 1.6,
                        brightness: 1.1,
                        ..blur(10.)
                    },
                ),
                3 => fx("lens frosted").apply(&mut scene, id, 0.4, false, now),
                4 => fx("fluted_glass frosted").apply(&mut scene, id, 0.4, false, now),
                _ => {
                    scene.set_effects(id, blur(10.));
                    scene.set_layer_shader(id, Some(mix.clone()));
                    scene.set_layer_uniforms(
                        id,
                        ShaderUniforms {
                            time: 0.,
                            params: [[0.; 4]; 4],
                        },
                    );
                }
            }
            id
        })
        .collect();
    let width = (size.0 * scale) as usize;
    // The bar's left edge: over each column's middle (56, 168, 280) in turn,
    // so each cell's middle changes from one frame to the next.
    let frames: Vec<Vec<u8>> = [41., 153., 265., 306.]
        .into_iter()
        .map(|x| {
            scene.set_transform(bar, Transform { x, y: 0. });
            let report = scene.flush();
            gpu.render(&scene, &report.damage).unwrap();
            gpu.readback().unwrap()
        })
        .collect();
    let at = |frame: &[u8], x: f32, y: f32| {
        let i = ((y * scale) as usize * width + (x * scale) as usize) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    };
    for (index, name) in BLUR_NAMES.iter().enumerate() {
        let (x, y) = (
            (index % columns) as f32 * cell.0 + cell.0 / 2.,
            (index / columns) as f32 * cell.1 + cell.1 / 2.,
        );
        let changes = frames
            .windows(2)
            .any(|pair| at(&pair[0], x, y) != at(&pair[1], x, y));
        assert!(
            changes,
            "{name}: doesn't blur the bar moving under it at {scale}x"
        );
    }
    let middle = at(&frames[0], cell.0 / 2., cell.1 / 2.);
    assert!(
        middle[3] == 255 && middle[..3] != [20, 24, 32],
        "the plain blur drew as its box opaque: {middle:?} at {scale}x"
    );
    drop(boxes);
    frames
}
#[cfg(windows)]
const BLUR_CELL: (f32, f32) = (112., 80.);
/// The blur frames' cells (`blur_frames`), for `where_`.
#[cfg(windows)]
const BLUR_NAMES: [&str; 6] = [
    "backdrop blur",
    "blur, edge fade",
    "blur, saturated",
    "lens frosted",
    "fluted_glass frosted",
    "glass over a blur",
];

/// Glass on a frame whose damage the scene's grid cuts (`CROSS_NAMES`): a
/// box reading a bar 140 points below it (`zgx_backdrop` at an offset, its
/// reach past it), a box beside the read from the bar's rows down, and 80
/// dots far off, all changing in the second frame: past `DAMAGE_RECTS`, the
/// damage goes on the grid, whose runs cut the read where the side box
/// starts. (The renderer draws the read as one region again,
/// `layer_shader.rs`'s `a_backdrop_read_is_never_split_between_regions`.)
/// The glass must show the bar's new colour.
#[cfg(windows)]
fn cross_region_frames(scale: f32) -> Vec<Vec<u8>> {
    use zgx::decoration::{FillShader, ShaderUniforms};
    use zgx::scene::*;
    let size = CROSS_SIZE;
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let fixed = |width: f32, height: f32| Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    };
    let reading = FillShader::new(
        "fn layer(input: LayerInput) -> vec4<f32> {
    let behind = zgx_backdrop(input.p + vec2(0., input.params[0].x));
    return vec4(mix(behind.rgb, input.color.rgb, 0.2), 1.);
}",
        None,
    );
    let bar = scene.append(
        root,
        NodeKind::Rect(Color(200, 40, 40, 255)),
        fixed(200., 20.),
    );
    scene.set_transform(bar, Transform { x: 40., y: 180. });
    let side = scene.append(
        root,
        NodeKind::Rect(Color(30, 60, 200, 255)),
        fixed(410., 300.),
    );
    scene.set_transform(side, Transform { x: 390., y: 176. });
    let glass = scene.append(
        root,
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: Color(20, 24, 32, 255),
                ..Default::default()
            },
        },
        fixed(200., 20.),
    );
    scene.set_transform(glass, Transform { x: 40., y: 40. });
    scene.set_layer_shader(glass, Some(reading));
    scene.set_layer_extent(glass, 0., 150.);
    let uniforms = |changed: f32| ShaderUniforms {
        time: 0.,
        params: [[140., changed, 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
    };
    scene.set_layer_uniforms(glass, uniforms(0.));
    let dots: Vec<NodeId> = (0..80)
        .map(|i| {
            let dot = scene.append(
                root,
                NodeKind::Rect(Color(250, 250, 250, 255)),
                fixed(2., 2.),
            );
            scene.set_transform(
                dot,
                Transform {
                    x: 1000. + (i % 10) as f32 * 8.,
                    y: 700. + (i / 10) as f32 * 8.,
                },
            );
            dot
        })
        .collect();
    let mut draw = |scene: &mut Scene| {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        (report.damage, gpu.readback().unwrap())
    };
    let (_, before) = draw(&mut scene);
    scene.set_kind(bar, NodeKind::Rect(Color(40, 200, 90, 255)));
    scene.set_kind(side, NodeKind::Rect(Color(200, 60, 30, 255)));
    for &dot in &dots {
        scene.set_kind(dot, NodeKind::Rect(Color(250, 200, 40, 255)));
    }
    scene.set_layer_uniforms(glass, uniforms(1.));
    let (damage, after) = draw(&mut scene);
    assert!(
        damage.iter().any(|d| d.y + d.height == 176.) && damage.iter().any(|d| d.y == 176.),
        "the damage isn't cut at the side box: {damage:?}"
    );
    let width = (size.0 * scale) as usize;
    let at = ((50. * scale) as usize * width + (140. * scale) as usize) * 4;
    // Mostly the bar it reads (a fifth the box): red, then green.
    let (was, now) = (&before[at..at + 4], &after[at..at + 4]);
    assert!(
        was[0] > 120 && was[1] < 80,
        "the glass doesn't show the bar: {was:?}"
    );
    assert!(
        now[1] > 120 && now[0] < 80,
        "the glass doesn't show the bar's new colour: {now:?}"
    );
    vec![before, after]
}
#[cfg(windows)]
const CROSS_SIZE: (f32, f32) = (1200., 900.);
/// The cross-region frames' parts (`cross_region_frames`), for `where_`.
#[cfg(windows)]
const CROSS_NAMES: [&str; 1] = ["glass on cut damage"];

/// A layer in each blend mode but `Normal` (`BLEND_NAMES`): a translucent
/// box holding a bar, over striped content, drawn first normally. Each must
/// change its cell.
#[cfg(windows)]
fn blend_frames(scale: f32) -> Vec<Vec<u8>> {
    use zgx::scene::*;
    let (cell, columns) = (LAYER_CELL, LAYER_COLUMNS);
    let size = (
        cell.0 * columns as f32,
        cell.1 * BLEND_NAMES.len().div_ceil(columns) as f32,
    );
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..(size.1 / 12.).ceil() as usize {
        let stripe = scene.append(
            root,
            NodeKind::Rect(match i % 3 {
                0 => Color(30, 60, 200, 255),
                1 => Color(230, 130, 30, 255),
                _ => Color(40, 170, 90, 160),
            }),
            Style {
                width: Some(size.0),
                height: Some(12.),
                ..Default::default()
            },
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 12.,
            },
        );
    }
    let boxes: Vec<NodeId> = (0..BLEND_NAMES.len())
        .map(|index| {
            let id = scene.append(
                root,
                NodeKind::Panel {
                    layout: Layout::Overlay,
                    quad: QuadStyle {
                        radius: 6.,
                        fill: Color(200, 80, 160, 190),
                        ..Default::default()
                    },
                },
                Style {
                    width: Some(56.),
                    height: Some(40.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                id,
                Transform {
                    x: (index % columns) as f32 * cell.0 + 20.,
                    y: (index / columns) as f32 * cell.1 + 16.,
                },
            );
            let bar = scene.append(
                id,
                NodeKind::Rect(Color(250, 240, 120, 255)),
                Style {
                    width: Some(28.),
                    height: Some(10.),
                    ..Default::default()
                },
            );
            scene.set_transform(bar, Transform { x: 14., y: 15. });
            // Isolated in both frames, so only the mode differs between them.
            scene.set_isolated(id, true);
            id
        })
        .collect();
    let mut draw = |scene: &mut Scene| {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        gpu.readback().unwrap()
    };
    let plain = draw(&mut scene);
    for (&id, (_, mode)) in boxes.iter().zip(BLEND_NAMES) {
        scene.set_blend(id, mode);
    }
    let blended = draw(&mut scene);
    let width = (size.0 * scale) as usize;
    for (index, (name, _)) in BLEND_NAMES.iter().enumerate() {
        let (x0, y0) = (
            ((index % columns) as f32 * cell.0 * scale) as usize,
            ((index / columns) as f32 * cell.1 * scale) as usize,
        );
        let (w, h) = ((cell.0 * scale) as usize, (cell.1 * scale) as usize);
        let changed = (y0..y0 + h).any(|y| {
            (x0..x0 + w)
                .any(|x| blended[(y * width + x) * 4..][..4] != plain[(y * width + x) * 4..][..4])
        });
        assert!(changed, "{name}: draws as normal");
    }
    vec![plain, blended]
}
/// The blend modes' cells (`blend_frames`), in their order.
#[cfg(windows)]
const BLEND_NAMES: [(&str, zgx::scene::BlendMode); 15] = {
    use zgx::scene::BlendMode::*;
    [
        ("multiply", Multiply),
        ("screen", Screen),
        ("overlay", Overlay),
        ("darken", Darken),
        ("lighten", Lighten),
        ("color dodge", ColorDodge),
        ("color burn", ColorBurn),
        ("hard light", HardLight),
        ("soft light", SoftLight),
        ("difference", Difference),
        ("exclusion", Exclusion),
        ("hue", Hue),
        ("saturation", Saturation),
        ("color", zgx::scene::BlendMode::Color),
        ("luminosity", Luminosity),
    ]
};

/// Images drawn with an effect (`IMAGE_EFFECT_NAMES`): a gradient image
/// dithered and as characters, at two sizes, one under a transform; plain
/// first. Each must change its cell.
#[cfg(windows)]
fn image_effect_frames(scale: f32) -> Vec<Vec<u8>> {
    use std::sync::Arc;
    use zgx::affine::Affine;
    use zgx::image::ImageData;
    use zgx::scene::*;
    let (cell, columns) = (LAYER_CELL, LAYER_COLUMNS);
    let size = (cell.0 * columns as f32, cell.1);
    let pixels: Vec<u8> = (0..32u32 * 32)
        .flat_map(|i| {
            let (x, y) = (i % 32, i / 32);
            [(x * 8) as u8, (y * 8) as u8, 160, 255]
        })
        .collect();
    let image = ImageData::new(32, 32, pixels).unwrap();
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    let nodes: Vec<NodeId> = (0..IMAGE_EFFECT_NAMES.len())
        .map(|index| {
            let id = scene.append(
                root,
                // Plain (no dot size), its pixels shared.
                NodeKind::Image(Arc::new(image.dithered(0.))),
                Style {
                    width: Some(48.),
                    height: Some(48.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                id,
                Transform {
                    x: index as f32 * cell.0 + 16.,
                    y: 12.,
                },
            );
            id
        })
        .collect();
    let mut draw = |scene: &mut Scene| {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        gpu.readback().unwrap()
    };
    let plain = draw(&mut scene);
    for (index, &id) in nodes.iter().enumerate() {
        let drawn = match IMAGE_EFFECT_NAMES[index] {
            "dithered 2" => image.dithered(2.),
            "dithered 3, scaled" => image.dithered(3.).transformed(Affine::scale(1.25, 1.25)),
            "characters 2" => image.characters(2.),
            "characters 3, scaled" => image.characters(3.).transformed(Affine::scale(1.25, 1.25)),
            name => unreachable!("{name}"),
        };
        scene.set_kind(id, NodeKind::Image(Arc::new(drawn)));
    }
    let effected = draw(&mut scene);
    let width = (size.0 * scale) as usize;
    for (index, name) in IMAGE_EFFECT_NAMES.iter().enumerate() {
        let x0 = (index as f32 * cell.0 * scale) as usize;
        let (w, h) = ((cell.0 * scale) as usize, (cell.1 * scale) as usize);
        let changed = (0..h).any(|y| {
            (x0..x0 + w)
                .any(|x| effected[(y * width + x) * 4..][..4] != plain[(y * width + x) * 4..][..4])
        });
        assert!(changed, "{name}: draws as the plain image");
    }
    vec![plain, effected]
}
/// The image effects' cells (`image_effect_frames`), in their order.
#[cfg(windows)]
const IMAGE_EFFECT_NAMES: [&str; 4] = [
    "dithered 2",
    "dithered 3, scaled",
    "characters 2",
    "characters 3, scaled",
];

/// Layer effects' cells (`EFFECT_NAMES`): a box holding a bar on striped
/// content, blurred (narrow, wide enough to be averaged down, and changing
/// sigma), masked by a ramp (still and sliding), and both, and casting a
/// drop shadow (hard, blurred, and under a content blur), with nothing
/// first and at a few moments after, as this process draws them at
/// `scale`. Each must change its cell: drawn as painted in both processes,
/// they would match.
#[cfg(windows)]
fn effect_frames(scale: f32) -> Vec<Vec<u8>> {
    use std::sync::Arc;
    use zgx::scene::*;
    let (cell, columns) = (LAYER_CELL, LAYER_COLUMNS);
    let size = (
        cell.0 * columns as f32,
        cell.1 * EFFECT_NAMES.len().div_ceil(columns) as f32,
    );
    // A ramp of 16 texels, clear to opaque, then a hard edge.
    let ramp: Vec<u8> = (0..16u32)
        .flat_map(|i| [255, 255, 255, (i * 17).min(255) as u8])
        .chain([255, 255, 255, 0])
        .collect();
    let ramp = Arc::new(zgx::image::ImageData::new(17, 1, ramp).unwrap());
    let mut gpu = GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    let mut scene = Scene::new(size.0, size.1);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    for i in 0..(size.1 / 12.).ceil() as usize {
        let stripe = scene.append(
            root,
            NodeKind::Rect(if i % 2 == 0 {
                Color(30, 60, 200, 255)
            } else {
                Color(230, 130, 30, 255)
            }),
            Style {
                width: Some(size.0),
                height: Some(12.),
                ..Default::default()
            },
        );
        scene.set_transform(
            stripe,
            Transform {
                x: 0.,
                y: i as f32 * 12.,
            },
        );
    }
    let boxes: Vec<NodeId> = (0..EFFECT_NAMES.len())
        .map(|index| {
            let id = scene.append(
                root,
                NodeKind::Panel {
                    layout: Layout::Overlay,
                    quad: QuadStyle {
                        radius: 6.,
                        fill: Color(20, 24, 32, 220),
                        ..Default::default()
                    },
                },
                Style {
                    width: Some(56.),
                    height: Some(40.),
                    ..Default::default()
                },
            );
            scene.set_transform(
                id,
                Transform {
                    x: (index % columns) as f32 * cell.0 + 20.,
                    y: (index / columns) as f32 * cell.1 + 16.,
                },
            );
            let bar = scene.append(
                id,
                NodeKind::Rect(Color(250, 250, 250, 255)),
                Style {
                    width: Some(28.),
                    height: Some(10.),
                    ..Default::default()
                },
            );
            scene.set_transform(bar, Transform { x: 14., y: 15. });
            id
        })
        .collect();
    let mut draw = |scene: &mut Scene| {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        gpu.readback().unwrap()
    };
    let plain = draw(&mut scene);
    let mut all = vec![plain.clone()];
    let mask = |offset: (f32, f32)| ImageMask {
        image: ramp.clone(),
        offset,
        size: (34., 40.),
    };
    for moment in 0..3 {
        let m = moment as f32;
        for (index, &id) in boxes.iter().enumerate() {
            match EFFECT_NAMES[index] {
                "blur 1.5" => scene.set_filter_blur(id, 1.5),
                "blur 6" => scene.set_filter_blur(id, 6.),
                "blur 24" => scene.set_filter_blur(id, 24.),
                "blur changing" => scene.set_filter_blur(id, 2. + 5. * m),
                "mask" => scene.set_mask(id, Some(mask((4., 0.)))),
                "mask sliding" => scene.set_mask(id, Some(mask((-6. + 9. * m, 0.)))),
                "mask under blur" => {
                    scene.set_filter_blur(id, 3. + m);
                    scene.set_mask(id, Some(mask((2. * m, 0.))));
                }
                "shadow" => scene.set_drop_shadow(
                    id,
                    Some(DropShadow {
                        offset: Transform { x: 4., y: 6. + m },
                        blur: 0.,
                        color: Color(0, 0, 0, 200),
                    }),
                ),
                "shadow blurred" => scene.set_drop_shadow(
                    id,
                    Some(DropShadow {
                        offset: Transform { x: 0., y: 8. },
                        blur: 4. + 6. * m,
                        color: Color(10, 200, 120, 255),
                    }),
                ),
                "shadow under blur" => {
                    scene.set_filter_blur(id, 2. + m);
                    scene.set_drop_shadow(
                        id,
                        Some(DropShadow {
                            offset: Transform { x: -3., y: 5. },
                            blur: 3.,
                            color: Color(120, 0, 160, 230),
                        }),
                    );
                }
                name => unreachable!("{name}"),
            }
        }
        all.push(draw(&mut scene));
    }
    let width = (size.0 * scale) as usize;
    for (index, name) in EFFECT_NAMES.iter().enumerate() {
        let (x0, y0) = (
            ((index % columns) as f32 * cell.0 * scale) as usize,
            ((index / columns) as f32 * cell.1 * scale) as usize,
        );
        let (w, h) = ((cell.0 * scale) as usize, (cell.1 * scale) as usize);
        let changed = all[1..].iter().all(|frame| {
            (y0..y0 + h).any(|y| {
                (x0..x0 + w)
                    .any(|x| frame[(y * width + x) * 4..][..4] != plain[(y * width + x) * 4..][..4])
            })
        });
        assert!(changed, "{name}: draws as painted at a moment");
    }
    all
}
/// The layer effects' cells (`effect_frames`), in their order.
#[cfg(windows)]
const EFFECT_NAMES: [&str; 10] = [
    "blur 1.5",
    "blur 6",
    "blur 24",
    "blur changing",
    "mask",
    "mask sliding",
    "mask under blur",
    "shadow",
    "shadow blurred",
    "shadow under blur",
];

/// The suite's view's frames at `scale`.
#[cfg(windows)]
fn suite_frames(scale: f32) -> Vec<Vec<u8>> {
    let mut gpu = GpuRenderer::new((SIZE.0 * scale) as u32, (SIZE.1 * scale) as u32).unwrap();
    gpu.set_scale_factor(scale);
    FontData::new(GEIST.to_vec())
        .unwrap()
        .install(&mut gpu.text_system().borrow_mut());
    let mut ui = Ui::new(SIZE.0, SIZE.1);
    gpu.install_text(&mut ui.scene.borrow_mut());
    let (live, offset) = (ui.signal(0_u32), ui.signal(0_f32));
    ui.mount(view(live.clone(), offset.clone()));
    (0..FRAMES)
        .map(|frame| {
            live.set(frame);
            // Whole pixels at the scale (scroll copies), and some not.
            offset.set(frame as f32 * if frame % 4 == 3 { 7.3 } else { 6. });
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            gpu.readback().unwrap()
        })
        .collect()
}

#[cfg(windows)]
fn main() {
    use std::io::{Read, Write};
    if std::env::var_os(CHILD).is_some() {
        let mut out = std::io::stdout().lock();
        for scale in SCALES {
            for frame in frames(scale) {
                out.write_all(&frame).unwrap();
            }
        }
        return;
    }
    let ours: Vec<(f32, Vec<Vec<u8>>)> = SCALES
        .into_iter()
        .map(|scale| (scale, frames(scale)))
        .collect();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .env("ZGX_D3D11", "1")
        .env(CHILD, "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut theirs = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut theirs)
        .unwrap();
    assert!(
        child.wait().unwrap().success(),
        "the Direct3D 11 run failed"
    );
    let expected: usize = ours
        .iter()
        .flat_map(|(_, frames)| frames)
        .map(Vec::len)
        .sum();
    assert_eq!(
        theirs.len(),
        expected,
        "the Direct3D 11 run drew too few frames"
    );
    let mut rest = &theirs[..];
    let mut failures = Vec::new();
    // Both processes draw on this adapter: WARP is a CPU device.
    let hardware = GpuRenderer::new(1, 1).unwrap().adapter_info().device_type
        != zgx_gpu::wgpu::DeviceType::Cpu;
    for (scale, frames) in &ours {
        for (frame, ours) in frames.iter().enumerate() {
            let (theirs, after) = rest.split_at(ours.len());
            rest = after;
            let (mut differ, mut most) = (0, 0);
            let mut first = Vec::new();
            let mut each = Vec::new();
            for (i, (a, b)) in ours.iter().zip(theirs).enumerate() {
                if a != b {
                    differ += 1;
                    most = most.max(a.abs_diff(*b));
                    if first.len() < 6 {
                        first.push(where_(frame, *scale, i / 4));
                    }
                    if each.len() < HARDWARE_ONE_OFF {
                        each.push((i, *a, *b));
                    }
                }
            }
            let allowed = hardware && app_shaders(frame) && most <= 1 && differ <= HARDWARE_ONE_OFF;
            if differ > 0 && allowed {
                for (i, a, b) in each {
                    println!(
                        "d3d11_parity {scale}×, frame {frame}: byte {} of {} is {a} on Direct3D 12, {b} on 11 (a hardware driver's rounding, allowed)",
                        i % 4,
                        where_(frame, *scale, i / 4)
                    );
                }
            } else if differ > 0 {
                failures.push(format!(
                    "{scale}×, frame {frame}: {differ} bytes differ between Direct3D 12 and 11, by up to {most}, at {first:?}"
                ));
            }
        }
        println!("d3d11_parity {scale}×: {} frames compared", frames.len());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// How many bytes of a frame of app shaders a hardware adapter may draw one
/// level apart (the module's doc): its drivers' rounding, never WARP's.
#[cfg(windows)]
const HARDWARE_ONE_OFF: usize = 4;

/// Whether frame `frame` is of app shaders, compiled for each API apart:
/// the fills' and the layer shaders'.
#[cfg(windows)]
fn app_shaders(frame: usize) -> bool {
    let frame = frame as u32;
    (FRAMES..FRAMES + 3 + 5).contains(&frame)
}

/// What pixel `pixel` of frame `frame` at `scale` shows: the suite's point,
/// or the fill or layer shader whose cell it lies in.
#[cfg(windows)]
fn where_(frame: usize, scale: f32, pixel: usize) -> String {
    let frame = frame as u32;
    let (width, cells, columns, names) = if frame < FRAMES {
        (SIZE.0, (0., 0.), 1, Vec::new())
    } else if frame < FRAMES + 3 {
        (
            FILL_CELL.0 * FILL_COLUMNS as f32,
            FILL_CELL,
            FILL_COLUMNS,
            fill_names(),
        )
    } else if frame < FRAMES + 3 + 5 {
        (
            LAYER_CELL.0 * LAYER_COLUMNS as f32,
            LAYER_CELL,
            LAYER_COLUMNS,
            layer_names(),
        )
    } else if frame < FRAMES + 3 + 5 + 4 {
        (
            LAYER_CELL.0 * LAYER_COLUMNS as f32,
            LAYER_CELL,
            LAYER_COLUMNS,
            EFFECT_NAMES.map(String::from).to_vec(),
        )
    } else if frame
        >= FRAMES + 3 + 5 + 4 + 2 + 2 + 5 + 2 + 3 + view_painters::VIEW_SCENE_NAMES.len() as u32 + 4
    {
        (FADE_SIZE.0, FADE_SIZE, 1, vec!["card in a fade".into()])
    } else if frame
        >= FRAMES + 3 + 5 + 4 + 2 + 2 + 5 + 2 + 3 + view_painters::VIEW_SCENE_NAMES.len() as u32
    {
        (
            BLUR_CELL.0 * 3.,
            BLUR_CELL,
            3,
            BLUR_NAMES.map(String::from).to_vec(),
        )
    } else if frame >= FRAMES + 3 + 5 + 4 + 2 + 2 + 5 + 2 + 3 {
        let name = view_painters::VIEW_SCENE_NAMES[(frame
            - (FRAMES + 3 + 5 + 4 + 2 + 2 + 5 + 2 + 3))
            as usize
            % view_painters::VIEW_SCENE_NAMES.len()];
        (
            view_painters::VIEW_SCENE_SIZE.0,
            view_painters::VIEW_SCENE_SIZE,
            1,
            vec![name.to_string()],
        )
    } else if frame >= FRAMES + 3 + 5 + 4 + 2 + 2 + 5 + 2 {
        (VIEW_SIZE.0, VIEW_SIZE, 1, vec!["GPU view".to_string()])
    } else if frame >= FRAMES + 3 + 5 + 4 + 2 + 2 + 5 {
        (
            CROSS_SIZE.0,
            CROSS_SIZE,
            1,
            CROSS_NAMES.map(String::from).to_vec(),
        )
    } else if frame >= FRAMES + 3 + 5 + 4 + 2 + 2 {
        (
            LAYER_CELL.0 * LAYER_COLUMNS as f32,
            LAYER_CELL,
            LAYER_COLUMNS,
            glass_names(),
        )
    } else if frame >= FRAMES + 3 + 5 + 4 + 2 {
        (
            LAYER_CELL.0 * LAYER_COLUMNS as f32,
            LAYER_CELL,
            LAYER_COLUMNS,
            BLEND_NAMES.map(|(name, _)| String::from(name)).to_vec(),
        )
    } else {
        (
            LAYER_CELL.0 * LAYER_COLUMNS as f32,
            LAYER_CELL,
            LAYER_COLUMNS,
            IMAGE_EFFECT_NAMES.map(String::from).to_vec(),
        )
    };
    let width = (width * scale) as usize;
    let (x, y) = (
        (pixel % width) as f32 / scale,
        (pixel / width) as f32 / scale,
    );
    if names.is_empty() {
        return format!("({x}, {y})");
    }
    let index = (y / cells.1) as usize * columns + (x / cells.0) as usize;
    format!(
        "{} ({x}, {y})",
        names.get(index).map_or("?", String::as_str)
    )
}
#[cfg(windows)]
const FILL_CELL: (f32, f32) = (56., 42.);
#[cfg(windows)]
const FILL_COLUMNS: usize = 7;
#[cfg(windows)]
const LAYER_CELL: (f32, f32) = (96., 72.);
#[cfg(windows)]
const LAYER_COLUMNS: usize = 6;
#[cfg(windows)]
fn fill_names() -> Vec<String> {
    let mut names: Vec<String> = zgx_fx::catalog()
        .into_iter()
        .filter(|(_, fx)| fx.kind() == zgx_fx::FxKind::Fill)
        .map(|(name, _)| name)
        .collect();
    names.push("radial".into());
    names
}
#[cfg(windows)]
fn glass_names() -> Vec<String> {
    let mut names: Vec<String> = zgx_fx::catalog()
        .into_iter()
        .filter(|(_, fx)| {
            fx.kind() == zgx_fx::FxKind::Layer && fx.frost() == 0. && fx.shader().reads_backdrop()
        })
        .map(|(name, _)| name)
        .collect();
    names.push("glass mix".into());
    names
}
#[cfg(windows)]
fn layer_names() -> Vec<String> {
    let mut names: Vec<String> = zgx_fx::catalog()
        .into_iter()
        .filter(|(_, fx)| {
            fx.kind() == zgx_fx::FxKind::Layer && fx.frost() == 0. && !fx.shader().reads_backdrop()
        })
        .map(|(name, _)| name)
        .collect();
    names.push("hue_rotate".into());
    names
}

#[cfg(not(windows))]
fn main() {}
