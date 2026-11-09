//! An app that goes wrong on purpose, for the host's tests: a key picks how.
//! `KeyT` panics (a trap), `KeyL` loops forever, `KeyM` takes memory until
//! it can't, `KeyE` exits with 7 (WASI 0.2 says only that it failed), `KeyF`
//! reads a file it has no folder for; on the GPU, at its next frame, `KeyG`
//! submits seconds of work (ten submissions of a hundred draws of a shader
//! looping two billion times a pixel, which the host's loop budget cuts
//! short; its device's watchdog stops the session) and `KeyD` records 200
//! draws in one submission. Its title counts
//! the events heard until then.
use zgx_app::{App, InputEvent, gpu, host, util};

/// The shader that keeps the GPU busy: a loop of dependent arithmetic per
/// pixel, its bound a uniform the compiler can't see through.
const STALL: &str = r#"
@group(0) @binding(0) var<uniform> bound: vec4<u32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var x = p.x * 0.001 + p.y * 0.0007;
    for (var i = 0u; i < bound.x; i++) {
        x = fract(x * 1.61803 + 0.31831);
    }
    return vec4<f32>(x, 0.0, 0.0, 1.0);
}
"#;

/// A shader with no bindings, for the probes of the caps.
const PLAIN: &str = r#"
@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }
"#;

/// What the next frame does on the GPU.
#[derive(Clone, Copy, PartialEq)]
enum Gpu {
    Nothing,
    Stall,
    ManyDraws,
    /// Two encoders of 80 draws each, submitted together.
    Split,
    /// An indirect dispatch.
    Indirect,
    /// Bundles past their caps, and bundle descriptors wgpu doesn't take.
    Bundles,
}

struct Misbehave {
    events: u32,
    hoard: Vec<Vec<u8>>,
    /// What its last try found (`KeyF`, `KeyD`).
    found: String,
    gpu: Gpu,
}

impl App for Misbehave {
    fn init(_info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        host::set_animating(false);
        host::set_title("events=0");
        Ok(Misbehave {
            events: 0,
            hoard: Vec::new(),
            found: String::new(),
            gpu: Gpu::Nothing,
        })
    }
    fn frame(&mut self, _time: f64) {
        let what = std::mem::replace(&mut self.gpu, Gpu::Nothing);
        if what == Gpu::Nothing {
            return;
        }
        self.found = match what {
            Gpu::Split => format!(" split={:?}", split()),
            Gpu::Indirect => format!(" indirect={:?}", indirect()),
            Gpu::Bundles => format!(" {}", bundles()),
            _ => format!(" submit={:?}", draw(what)),
        };
        host::set_title(&format!("events={}{}", self.events, self.found));
    }
    fn event(&mut self, event: InputEvent) {
        self.events += 1;
        host::set_title(&format!("events={}{}", self.events, self.found));
        let InputEvent::Key(key) = event else { return };
        if !key.down {
            return;
        }
        match key.code.as_str() {
            "KeyT" => panic!("a trap on purpose"),
            "KeyL" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "KeyM" => loop {
                let mut block = vec![0u8; 1 << 20];
                block[0] = 1;
                self.hoard.push(std::hint::black_box(block));
            },
            "KeyE" => std::process::exit(7),
            "KeyG" => {
                self.gpu = Gpu::Stall;
                host::request_frame();
            }
            "KeyD" => {
                self.gpu = Gpu::ManyDraws;
                host::request_frame();
            }
            "KeyK" => {
                self.gpu = Gpu::Split;
                host::request_frame();
            }
            "KeyI" => {
                self.gpu = Gpu::Indirect;
                host::request_frame();
            }
            "KeyB" => {
                self.gpu = Gpu::Bundles;
                host::request_frame();
            }
            "KeyF" => {
                let read = std::fs::read_to_string("/etc/passwd").map(|s| s.len());
                let listed = std::fs::read_dir("/").map(|d| d.count());
                self.found = format!(" file={read:?} dir={listed:?}");
                host::set_title(&format!("events={}{}", self.events, self.found));
            }
            _ => {}
        }
    }
}

zgx_app::app!(Misbehave);

/// One frame's GPU work: the stalling shader once, or a cheap one 200 times.
fn draw(what: Gpu) -> Result<(), String> {
    let module = gpu::create_shader_module(Some("stall"), STALL)?;
    let pipeline = gpu::create_render_pipeline(&gpu::RenderPipelineDescriptor {
        label: Some("stall".into()),
        layout: None,
        vertex: gpu::VertexState {
            module: &module,
            entry_point: Some("vs".into()),
            buffers: Vec::new(),
            constants: Vec::new(),
        },
        primitive: util::primitive(gpu::PrimitiveTopology::TriangleList, gpu::CullMode::None),
        depth_stencil: None,
        multisample: util::multisample(1),
        fragment: Some(gpu::FragmentState {
            module: &module,
            entry_point: Some("fs".into()),
            targets: vec![Some(util::target(gpu::TextureFormat::Bgra8unorm))],
            constants: Vec::new(),
        }),
    })?;
    let bound: u32 = if what == Gpu::Stall { 2_000_000_000 } else { 1 };
    let mut contents = Vec::new();
    for word in [bound, 0, 0, 0] {
        contents.extend_from_slice(&word.to_le_bytes());
    }
    let uniforms =
        gpu::create_buffer_with_data(Some("bound"), gpu::BufferUsage::UNIFORM, &contents)?;
    let layout = pipeline.get_bind_group_layout(0)?;
    let group = gpu::create_bind_group(
        Some("stall"),
        &layout,
        &[gpu::BindGroupEntry {
            binding: 0,
            resource: gpu::BindingResource::Buffer(gpu::BufferBinding {
                buffer: &uniforms,
                offset: 0,
                size: None,
            }),
        }],
    )?;
    let target = gpu::current_texture()?;
    let view = target.create_view(None)?;
    let (submissions, draws) = if what == Gpu::Stall {
        (10, 100)
    } else {
        (1, 200)
    };
    for _ in 0..submissions {
        let encoder = gpu::create_command_encoder(Some("stall"))?;
        {
            let pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
                label: None,
                color_attachments: vec![Some(gpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    clear: Some(gpu::Color {
                        r: 0.,
                        g: 0.,
                        b: 0.,
                        a: 1.,
                    }),
                    store: gpu::StoreOp::Store,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            })?;
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, Some(&group), &[]);
            for _ in 0..draws {
                pass.draw(3, 1, 0, 0);
            }
            pass.end();
        }
        gpu::submit(vec![gpu::CommandEncoder::finish(encoder)?])?;
    }
    Ok(())
}

/// A pipeline drawing into a frame's texture, and that texture's view.
fn setup() -> Result<(gpu::RenderPipeline, gpu::TextureView), String> {
    let module = gpu::create_shader_module(Some("plain"), PLAIN)?;
    let pipeline = gpu::create_render_pipeline(&gpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: gpu::VertexState {
            module: &module,
            entry_point: Some("vs".into()),
            buffers: Vec::new(),
            constants: Vec::new(),
        },
        primitive: util::primitive(gpu::PrimitiveTopology::TriangleList, gpu::CullMode::None),
        depth_stencil: None,
        multisample: util::multisample(1),
        fragment: Some(gpu::FragmentState {
            module: &module,
            entry_point: Some("fs".into()),
            targets: vec![Some(util::target(gpu::TextureFormat::Bgra8unorm))],
            constants: Vec::new(),
        }),
    })?;
    let view = gpu::current_texture()?.create_view(None)?;
    Ok((pipeline, view))
}

/// `draws` draws recorded in a new encoder, finished.
fn record(
    pipeline: &gpu::RenderPipeline,
    view: &gpu::TextureView,
    draws: u32,
) -> Result<gpu::CommandBuffer, String> {
    let encoder = gpu::create_command_encoder(None)?;
    {
        let pass = encoder.begin_render_pass(&gpu::RenderPassDescriptor {
            label: None,
            color_attachments: vec![Some(gpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                clear: None,
                store: gpu::StoreOp::Store,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        })?;
        pass.set_pipeline(pipeline);
        for _ in 0..draws {
            pass.draw(0, 1, 0, 0);
        }
        pass.end();
    }
    gpu::CommandEncoder::finish(encoder)
}

/// Two encoders under the cap each, over it together.
fn split() -> Result<(), String> {
    let (pipeline, view) = setup()?;
    let first = record(&pipeline, &view, 80)?;
    let second = record(&pipeline, &view, 80)?;
    gpu::submit(vec![first, second])
}

/// An indirect dispatch, whose size the GPU reads.
fn indirect() -> Result<(), String> {
    let module =
        gpu::create_shader_module(Some("indirect"), "@compute @workgroup_size(1) fn main() {}")?;
    let pipeline = gpu::create_compute_pipeline(&gpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main".into()),
        constants: Vec::new(),
    })?;
    let mut args = Vec::new();
    for n in [65535u32, 65535, 65535] {
        args.extend_from_slice(&n.to_le_bytes());
    }
    let buffer = gpu::create_buffer_with_data(None, gpu::BufferUsage::INDIRECT, &args)?;
    let encoder = gpu::create_command_encoder(None)?;
    {
        let pass = encoder.begin_compute_pass(None, None)?;
        pass.set_pipeline(&pipeline);
        pass.dispatch_workgroups_indirect(&buffer, 0);
        pass.end();
    }
    gpu::submit(vec![gpu::CommandEncoder::finish(encoder)?])
}

/// Each refused, never a panic: too many draws, too many commands, and
/// descriptors wgpu doesn't take.
fn bundles() -> String {
    let encoder =
        |samples: u32, color: Option<gpu::TextureFormat>, depth: Option<gpu::TextureFormat>| {
            gpu::create_render_bundle_encoder(&gpu::RenderBundleEncoderDescriptor {
                label: None,
                color_formats: vec![color],
                depth_stencil: depth.map(|format| gpu::RenderBundleDepthStencil {
                    format,
                    depth_read_only: false,
                    stencil_read_only: false,
                }),
                sample_count: samples,
            })
        };
    let bgra = Some(gpu::TextureFormat::Bgra8unorm);
    let result = |r: Result<gpu::RenderBundle, String>| match r {
        Ok(_) => "made".to_owned(),
        Err(why) => format!("refused({why})"),
    };
    let Ok((pipeline, _view)) = setup() else {
        return "setup-failed".into();
    };
    let draws = encoder(1, bgra, None).and_then(|e| {
        e.set_pipeline(&pipeline);
        for _ in 0..200 {
            e.draw(0, 1, 0, 0);
        }
        gpu::RenderBundleEncoder::finish(e, None)
    });
    let commands = encoder(1, bgra, None).and_then(|e| {
        for _ in 0..70_000 {
            e.set_pipeline(&pipeline);
        }
        gpu::RenderBundleEncoder::finish(e, None)
    });
    let samples = encoder(3, bgra, None).and_then(|e| gpu::RenderBundleEncoder::finish(e, None));
    let depth = encoder(1, bgra, Some(gpu::TextureFormat::Rgba8unorm))
        .and_then(|e| gpu::RenderBundleEncoder::finish(e, None));
    let color = encoder(1, Some(gpu::TextureFormat::Depth32float), None)
        .and_then(|e| gpu::RenderBundleEncoder::finish(e, None));
    format!(
        "bundle-draws={} bundle-commands={} bundle-samples={} bundle-depth={} bundle-color={}",
        result(draws),
        result(commands),
        result(samples),
        result(depth),
        result(color)
    )
}
