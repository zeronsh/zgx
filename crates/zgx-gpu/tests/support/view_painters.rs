//! GPU views (`zgx::gpu_view`) drawn by small wgpu painters, in seven
//! scenes, for comparing how a host shows them (Direct3D 11's view host
//! against Direct3D 12, `d3d11_parity.rs`; Metal, `view_painters.rs`):
//!
//! 1. a lit, textured cube with depth;
//! 2. spokes drawn with 4x MSAA, resolved into the view's target;
//! 3. a view clear but for a soft disc (premultiplied), over a bar of UI;
//! 4. one view whose box changes size between frames;
//! 5. one view animated across frames by a counter;
//! 6. two views in one window;
//! 7. a view under a parent that clips with a rounded corner, turns,
//!    scales and is translucent.
//!
//! Every painter is deterministic (no clock, no randomness: what moves is a
//! counter the scene sets), keeps its pipeline per device and drops it in
//! `device_changed`. Each scene asserts its view drew: a view left out would
//! compare equal on both sides of a parity test. Only wgpu, zgx and
//! zgx-gpu.
#![allow(dead_code)]
use std::{cell::Cell, rc::Rc};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_gpu::{
    GpuRenderer,
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

/// Every scene's window, in points.
pub const VIEW_SCENE_SIZE: (f32, f32) = (240., 160.);
/// Each frame `view_scene_frames` returns, in order.
pub const VIEW_SCENE_NAMES: [&str; 9] = [
    "lit textured cube with depth",
    "MSAA resolved into the target",
    "transparent view over UI",
    "resized: first size",
    "resized: second size",
    "animated: frame 0",
    "animated: frame 1",
    "two views",
    "clipped, rounded, turned, scaled, translucent",
];
/// The UI behind every view.
const BACKGROUND: Color = Color(20, 24, 32, 255);
const BAR: Color = Color(230, 130, 30, 255);

/// A render pipeline drawing `vertices` from `wgsl` (`vs`, `fs`) into
/// `format`, with depth and samples as asked.
fn pipeline(
    device: &wgpu::Device,
    wgsl: &str,
    format: wgpu::TextureFormat,
    depth: bool,
    samples: u32,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("view painter"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("view painter"),
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
        depth_stencil: depth.then(|| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

/// A texture the size of the view's target, remade when that changes.
fn sized(
    kept: &mut Option<((u32, u32), wgpu::TextureView)>,
    device: &wgpu::Device,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::TextureView {
    if kept.as_ref().is_none_or(|(at, _)| *at != size) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("view painter target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        *kept = Some((size, texture.create_view(&Default::default())));
    }
    kept.as_ref().unwrap().1.clone()
}

fn attachment<'a>(
    view: &'a wgpu::TextureView,
    resolve: Option<&'a wgpu::TextureView>,
    clear: wgpu::Color,
) -> wgpu::RenderPassColorAttachment<'a> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: resolve,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        },
    }
}

// 1. A cube, turned, textured with a checker and lit from one side, its
// faces sorted by a depth buffer of the view's size.
const CUBE: &str = "
struct Out { @builtin(position) pos: vec4f, @location(0) n: vec3f, @location(1) uv: vec2f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    var faces = array(vec3f(1., 0., 0.), vec3f(-1., 0., 0.), vec3f(0., 1., 0.),
                      vec3f(0., -1., 0.), vec3f(0., 0., 1.), vec3f(0., 0., -1.));
    var quad = array(vec2f(-1., -1.), vec2f(1., -1.), vec2f(1., 1.),
                     vec2f(-1., -1.), vec2f(1., 1.), vec2f(-1., 1.));
    let n = faces[i / 6u];
    let u = vec3f(n.y, n.z, n.x);
    let v = cross(n, u);
    let q = quad[i % 6u];
    let p = (n + u * q.x + v * q.y) * 0.5;
    let a = 0.6;
    let b = 0.45;
    let ry = mat3x3f(vec3f(cos(a), 0., -sin(a)), vec3f(0., 1., 0.), vec3f(sin(a), 0., cos(a)));
    let rx = mat3x3f(vec3f(1., 0., 0.), vec3f(0., cos(b), sin(b)), vec3f(0., -sin(b), cos(b)));
    let w = rx * ry * p;
    let z = w.z + 2.5;
    var out: Out;
    out.pos = vec4f(w.x * 2.4, w.y * 3.2, z - 1.5, z);
    out.n = rx * ry * n;
    out.uv = q * 0.5 + 0.5;
    return out;
}
@group(0) @binding(0) var map: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;
@fragment fn fs(in: Out) -> @location(0) vec4f {
    let lit = 0.25 + 0.75 * max(dot(normalize(in.n), normalize(vec3f(-0.4, 0.6, -0.7))), 0.);
    return vec4f(textureSample(map, nearest, in.uv).rgb * lit, 1.);
}";

#[derive(Default)]
pub struct LitCube {
    pipeline: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
    depth: Option<((u32, u32), wgpu::TextureView)>,
}
impl Viewport for LitCube {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let (device, queue, format) = (frame.device, frame.queue, frame.format);
        let (pipeline, bind) = self.pipeline.get_or_insert_with(|| {
            let pipeline = pipeline(device, CUBE, format, true, 1);
            // An 8 by 8 checker of two colours.
            let texels: Vec<u8> = (0..64)
                .flat_map(|i| {
                    if (i % 8 + i / 8) % 2 == 0 {
                        [220, 200, 160, 255]
                    } else {
                        [60, 110, 200, 255]
                    }
                })
                .collect();
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("checker"),
                size: wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
            );
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("checker"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &texture.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            &device.create_sampler(&Default::default()),
                        ),
                    },
                ],
            });
            (pipeline, bind)
        });
        let depth = sized(
            &mut self.depth,
            device,
            frame.size,
            wgpu::TextureFormat::Depth32Float,
            1,
        );
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lit cube"),
                color_attachments: &[Some(attachment(
                    frame.target,
                    None,
                    wgpu::Color {
                        r: 0.15,
                        g: 0.15,
                        b: 0.2,
                        a: 1.,
                    },
                ))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*bind, &[]);
        pass.draw(0..36, 0..1);
    }
    fn device_changed(&mut self) {
        self.pipeline = None;
        self.depth = None;
    }
}

// 2. Six thin spokes at 4x MSAA, resolved into the target.
const SPOKES: &str = "
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let spoke = f32(i / 3u) * 0.5236;
    let corner = i % 3u;
    let d = vec2f(cos(spoke), sin(spoke));
    let side = vec2f(-d.y, d.x) * 0.04;
    var p = vec2f(0.);
    if corner == 1u { p = d * 0.9 + side; }
    if corner == 2u { p = d * 0.9 - side; }
    return vec4f(p, 0., 1.);
}
@fragment fn fs() -> @location(0) vec4f { return vec4f(0.95, 0.95, 0.9, 1.); }";

#[derive(Default)]
pub struct Spokes {
    pipeline: Option<wgpu::RenderPipeline>,
    samples: Option<((u32, u32), wgpu::TextureView)>,
}
impl Viewport for Spokes {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let (device, format) = (frame.device, frame.format);
        let pipeline = self
            .pipeline
            .get_or_insert_with(|| pipeline(device, SPOKES, format, false, 4));
        let samples = sized(&mut self.samples, device, frame.size, format, 4);
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spokes"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.4,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                    ..attachment(&samples, Some(frame.target), wgpu::Color::BLACK)
                })],
                ..Default::default()
            });
        pass.set_pipeline(pipeline);
        pass.draw(0..18, 0..1);
    }
    fn device_changed(&mut self) {
        self.pipeline = None;
        self.samples = None;
    }
}

// 3. Clear everywhere but a soft-edged disc, premultiplied: what's behind
// the view shows through.
const DISC: &str = "
struct Out { @builtin(position) pos: vec4f, @location(0) at: vec2f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    var p = array(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    var out: Out;
    out.pos = vec4f(p[i], 0., 1.);
    out.at = p[i];
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4f {
    let a = 1. - smoothstep(0.4, 0.65, length(in.at));
    return vec4f(vec3f(0.2, 0.5, 0.95) * a, a);
}";

#[derive(Default)]
pub struct Disc {
    pipeline: Option<wgpu::RenderPipeline>,
}
impl Viewport for Disc {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let (device, format) = (frame.device, frame.format);
        let pipeline = self
            .pipeline
            .get_or_insert_with(|| pipeline(device, DISC, format, false, 1));
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("disc"),
                color_attachments: &[Some(attachment(
                    frame.target,
                    None,
                    wgpu::Color::TRANSPARENT,
                ))],
                ..Default::default()
            });
        pass.set_pipeline(pipeline);
        pass.draw(0..3, 0..1);
    }
    fn device_changed(&mut self) {
        self.pipeline = None;
    }
}

// 4, 6, 7. Stripes in a colour of its own, from the pixel's place: a box of
// another size shows another picture.
const STRIPES: &str = "
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    var p = array(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    return vec4f(p[i], 0., 1.);
}
@group(0) @binding(0) var<uniform> tint: vec4f;
@fragment fn fs(@builtin(position) at: vec4f) -> @location(0) vec4f {
    let stripe = select(0.7, 1., (u32(at.x / 12.) + u32(at.y / 12.)) % 2u == 0u);
    return vec4f(tint.rgb * stripe, 1.);
}";

pub struct Stripes {
    tint: [f32; 4],
    pipeline: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
}
impl Stripes {
    pub fn new(tint: [f32; 3]) -> Self {
        Self {
            tint: [tint[0], tint[1], tint[2], 1.],
            pipeline: None,
        }
    }
}
impl Viewport for Stripes {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let (device, format) = (frame.device, frame.format);
        let tint = self.tint;
        let (pipeline, bind) = self.pipeline.get_or_insert_with(|| {
            let pipeline = pipeline(device, STRIPES, format, false, 1);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tint"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: true,
            });
            buffer.slice(..).get_mapped_range_mut().copy_from_slice(
                &tint
                    .iter()
                    .flat_map(|c| c.to_le_bytes())
                    .collect::<Vec<u8>>(),
            );
            buffer.unmap();
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tint"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (pipeline, bind)
        });
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stripes"),
                color_attachments: &[Some(attachment(frame.target, None, wgpu::Color::BLACK))],
                ..Default::default()
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*bind, &[]);
        pass.draw(0..3, 0..1);
    }
    fn device_changed(&mut self) {
        self.pipeline = None;
    }
}

// 5. A triangle turned a step a frame: the step is a counter the scene
// sets, written to a uniform each frame.
const TURNING: &str = "
@group(0) @binding(0) var<uniform> step: vec4f;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let a = step.x * 0.7 + f32(i) * 2.0944;
    return vec4f(cos(a) * 0.8, sin(a) * 0.8, 0., 1.);
}
@fragment fn fs() -> @location(0) vec4f { return vec4f(0.95, 0.4, 0.3, 1.); }";

pub struct Turning {
    step: Rc<Cell<u32>>,
    pipeline: Option<(wgpu::RenderPipeline, wgpu::Buffer, wgpu::BindGroup)>,
}
impl Turning {
    pub fn new(step: Rc<Cell<u32>>) -> Self {
        Self {
            step,
            pipeline: None,
        }
    }
}
impl Viewport for Turning {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let (device, format) = (frame.device, frame.format);
        let (pipeline, buffer, bind) = self.pipeline.get_or_insert_with(|| {
            let pipeline = pipeline(device, TURNING, format, false, 1);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("step"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("step"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (pipeline, buffer, bind)
        });
        let step = [self.step.get() as f32, 0., 0., 0.];
        frame.queue.write_buffer(
            buffer,
            0,
            &step
                .iter()
                .flat_map(|c| c.to_le_bytes())
                .collect::<Vec<u8>>(),
        );
        let mut pass = frame
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("turning"),
                color_attachments: &[Some(attachment(
                    frame.target,
                    None,
                    wgpu::Color {
                        r: 0.12,
                        g: 0.12,
                        b: 0.12,
                        a: 1.,
                    },
                ))],
                ..Default::default()
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*bind, &[]);
        pass.draw(0..3, 0..1);
    }
    fn device_changed(&mut self) {
        self.pipeline = None;
    }
}

/// A window of `VIEW_SCENE_SIZE` at `scale`, or `None` where there's no
/// GPU (and none is required).
fn window(scale: f32) -> Option<(Ui, GpuRenderer)> {
    let size = VIEW_SCENE_SIZE;
    let gpu = match GpuRenderer::new((size.0 * scale) as u32, (size.1 * scale) as u32) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return None,
    };
    let mut gpu = gpu;
    gpu.set_scale_factor(scale);
    Some((Ui::new(size.0, size.1), gpu))
}
fn draw(ui: &mut Ui, gpu: &mut GpuRenderer) -> Vec<u8> {
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    gpu.readback().unwrap()
}
/// The pixel at (`x`, `y`) points of a frame at `scale`, RGBA.
fn pixel(frame: &[u8], scale: f32, x: f32, y: f32) -> [u8; 4] {
    let width = (VIEW_SCENE_SIZE.0 * scale) as usize;
    let at = ((y * scale) as usize * width + (x * scale) as usize) * 4;
    frame[at..at + 4].try_into().unwrap()
}
fn near(got: [u8; 4], want: [u8; 3], within: u8) -> bool {
    (0..3).all(|c| got[c].abs_diff(want[c]) <= within)
}
/// The window behind a scene's views: its background and, across its
/// middle, a bar.
fn backdrop() -> View {
    div()
        .size(VIEW_SCENE_SIZE.0, VIEW_SCENE_SIZE.1)
        .bg(BACKGROUND)
        .child(div().absolute().ml(10.).mt(70.).size(220., 20.).bg(BAR))
}

/// Each scene's frames (`VIEW_SCENE_NAMES`), RGBA, as this process draws
/// them at `scale`; empty where there's no GPU. Panics if a view didn't
/// draw.
pub fn view_scene_frames(scale: f32) -> Vec<Vec<u8>> {
    let Some(_) = window(scale) else {
        return Vec::new();
    };
    let mut all = Vec::new();
    let background = [BACKGROUND.0, BACKGROUND.1, BACKGROUND.2];
    let bar = [BAR.0, BAR.1, BAR.2];

    // 1. The cube in the middle; around it, the view's clear colour.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let view = zgx_gpu::viewport::view(LitCube::default());
        ui.mount(
            backdrop().child(
                gpu_view("lit", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .size(160., 120.),
            ),
        );
        let frame = draw(&mut ui, &mut gpu);
        let middle = pixel(&frame, scale, 120., 80.);
        assert!(
            !near(middle, [38, 38, 51], 4) && !near(middle, bar, 4),
            "cube not drawn: {middle:?}"
        );
        assert!(
            near(pixel(&frame, scale, 44., 24.), [38, 38, 51], 3),
            "view's clear not drawn"
        );
        all.push(frame);
    }
    // 2. The spokes' edges are a mix of their colour and the clear: the
    // samples were resolved.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let view = zgx_gpu::viewport::view(Spokes::default());
        ui.mount(
            backdrop().child(
                gpu_view("msaa", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .size(160., 120.),
            ),
        );
        let frame = draw(&mut ui, &mut gpu);
        let (spoke, clear): ([u8; 3], [u8; 3]) = ([242, 242, 230], [26, 51, 102]);
        let (mut mixed, mut solid) = (0, 0);
        for y in 20..140 {
            for x in 40..200 {
                let p = pixel(&frame, scale, x as f32, y as f32);
                let between = (0..3).all(|c| {
                    p[c] >= clear[c].min(spoke[c]).saturating_sub(3)
                        && p[c] <= clear[c].max(spoke[c]).saturating_add(3)
                });
                if near(p, spoke, 3) {
                    solid += 1;
                } else if between && !near(p, clear, 3) {
                    mixed += 1;
                }
            }
        }
        assert!(
            solid > 0 && mixed > 0,
            "spokes not drawn and resolved: {solid} solid, {mixed} mixed"
        );
        all.push(frame);
    }
    // 3. The disc covers the bar; beside it, the bar shows through.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let view = zgx_gpu::viewport::view(Disc::default());
        ui.mount(
            backdrop().child(
                gpu_view("transparent", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .size(160., 120.),
            ),
        );
        let frame = draw(&mut ui, &mut gpu);
        assert!(
            near(pixel(&frame, scale, 120., 80.), [51, 128, 242], 6),
            "disc not drawn"
        );
        assert!(
            near(pixel(&frame, scale, 190., 80.), bar, 3),
            "the bar doesn't show through"
        );
        assert!(
            near(pixel(&frame, scale, 190., 30.), background, 3),
            "the background doesn't show through"
        );
        all.push(frame);
    }
    // 4. The same view, its box 120 wide, then 180: its stripes reach the
    // new width.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let width = ui.signal(120.);
        let w = width.clone();
        let view = zgx_gpu::viewport::view(Stripes::new([0.3, 0.8, 0.4]));
        ui.mount(
            backdrop().child(
                gpu_view("resized", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .h(120.)
                    .reactive_style(move || Styles::new().w(w.get())),
            ),
        );
        let first = draw(&mut ui, &mut gpu);
        assert!(
            near(pixel(&first, scale, 190., 30.), background, 3),
            "the first box reaches too far"
        );
        width.set(180.);
        let second = draw(&mut ui, &mut gpu);
        let p = pixel(&second, scale, 210., 30.);
        assert!(
            p[1] > p[0] + 40 && p[1] > p[2] + 40,
            "the second box isn't drawn to its width: {p:?}"
        );
        all.push(first);
        all.push(second);
    }
    // 5. The triangle in another place each frame.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let step = Rc::new(Cell::new(0));
        let view = zgx_gpu::viewport::view(Turning::new(step.clone()));
        ui.mount(
            backdrop().child(
                gpu_view("animated", &view)
                    .absolute()
                    .ml(40.)
                    .mt(20.)
                    .size(160., 120.),
            ),
        );
        let first = draw(&mut ui, &mut gpu);
        step.set(1);
        view.invalidate();
        let second = draw(&mut ui, &mut gpu);
        assert_ne!(first, second, "the view didn't move between frames");
        assert!(
            [&first, &second].iter().all(|f| {
                (20..140).any(|y| {
                    (40..200).any(|x| near(pixel(f, scale, x as f32, y as f32), [242, 102, 77], 4))
                })
            }),
            "the triangle isn't drawn in every frame"
        );
        all.push(first);
        all.push(second);
    }
    // 6. Two views side by side, each its own colour.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let left = zgx_gpu::viewport::view(Stripes::new([0.9, 0.2, 0.2]));
        let right = zgx_gpu::viewport::view(Stripes::new([0.2, 0.3, 0.9]));
        ui.mount(
            backdrop()
                .child(
                    gpu_view("left", &left)
                        .absolute()
                        .ml(20.)
                        .mt(30.)
                        .size(90., 100.),
                )
                .child(
                    gpu_view("right", &right)
                        .absolute()
                        .ml(130.)
                        .mt(30.)
                        .size(90., 100.),
                ),
        );
        let frame = draw(&mut ui, &mut gpu);
        let (l, r) = (
            pixel(&frame, scale, 65., 50.),
            pixel(&frame, scale, 175., 50.),
        );
        assert!(
            l[0] > l[2] + 60 && r[2] > r[0] + 60,
            "two views not both drawn: {l:?} {r:?}"
        );
        all.push(frame);
    }
    // 7. A view larger than its parent, which clips it with a rounded
    // corner, turns and scales it, at 0.7 opacity.
    {
        let (mut ui, mut gpu) = window(scale).unwrap();
        let view = zgx_gpu::viewport::view(Stripes::new([0.95, 0.85, 0.2]));
        ui.mount(
            backdrop().child(
                div()
                    .absolute()
                    .ml(50.)
                    .mt(30.)
                    .size(140., 100.)
                    .overflow_hidden()
                    .rounded(30.)
                    .rotate(0.15)
                    .scale(0.9)
                    .opacity(0.7)
                    .child(
                        gpu_view("clipped", &view)
                            .absolute()
                            .ml(-20.)
                            .mt(-20.)
                            .size(180., 140.),
                    ),
            ),
        );
        let frame = draw(&mut ui, &mut gpu);
        let middle = pixel(&frame, scale, 120., 50.);
        assert!(
            middle[0] > 120 && middle[1] > 100 && middle[2] < 90 && middle[0] < 245,
            "the view isn't drawn translucent in its parent: {middle:?}"
        );
        // Past the parent's rounded, turned corner: the window's background.
        assert!(
            near(pixel(&frame, scale, 52., 32.), background, 3),
            "the view isn't clipped at its corner"
        );
        assert!(
            near(pixel(&frame, scale, 30., 20.), background, 3),
            "the view reaches past its parent"
        );
        all.push(frame);
    }
    assert_eq!(all.len(), VIEW_SCENE_NAMES.len());
    all
}
