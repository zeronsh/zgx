//! The app's post passes (`World::add_post`) on the device: a pipeline a
//! pass (and target format), its parameters' buffer, and its bindings of
//! the chain's targets. HDR passes trade the scene back and forth with its
//! other half, ending in the scene (an odd count copies back once); display
//! passes trade what the display shows, the last writing the view's target.
use crate::world::{AppPass, PostPassId, PostStage};
use std::collections::HashMap;
use std::time::Instant;
use zgx_gpu::wgpu;

/// A pass's frame (the prelude's `ZgxFrame`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Frame {
    time: f32,
    delta: f32,
    index: u32,
    pad: u32,
}

pub(super) struct Apps {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    frame: wgpu::Buffer,
    modules: HashMap<PostPassId, wgpu::ShaderModule>,
    /// By pass, target format, and whether it encodes sRGB itself.
    pipelines: HashMap<(PostPassId, wgpu::TextureFormat, bool), wgpu::RenderPipeline>,
    params: HashMap<PostPassId, wgpu::Buffer>,
    /// Passes the device refused (their shader or pipeline), with why: not
    /// run, not tried again.
    pub(super) failed: HashMap<PostPassId, String>,
    /// By pass and the texture it reads (0 scene, 1 its other half, 2 what
    /// the display shows, 3 its other half), for `compiled`.
    binds: HashMap<(PostPassId, usize), wgpu::BindGroup>,
    /// The scene's other half, for the copy back into the scene.
    copy_bind: Option<wgpu::BindGroup>,
    compiled: u64,
    /// The first frame's time and the last's (`ViewportFrame::time`).
    clock: Option<(Instant, Instant)>,
}

/// Which of the graph's views a pass reads or writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Scene,
    Scene2,
    Shown,
    Shown2,
}

impl Apps {
    #[cold]
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D post, app"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(2),
                uniform(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D post, app"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            layout,
            pipeline_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("3D post, app"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            frame: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D post, app frame"),
                size: std::mem::size_of::<Frame>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            modules: HashMap::new(),
            pipelines: HashMap::new(),
            params: HashMap::new(),
            failed: HashMap::new(),
            binds: HashMap::new(),
            copy_bind: None,
            compiled: 0,
            clock: None,
        }
    }

    /// Lets go of what holds the graph's targets.
    pub(super) fn rest(&mut self) {
        self.binds.clear();
        self.copy_bind = None;
    }

    /// `passes` of `stage` run in turn: HDR passes on the scene, display
    /// passes on what the display shows, the last into `target` (of
    /// `format`, encoded by the pass where `encode`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &super::Graph,
        passes: &[AppPass],
        stage: PostStage,
        (target, format, encode): (&wgpu::TextureView, wgpu::TextureFormat, bool),
        (index, now, copy): (u64, Instant, &wgpu::RenderPipeline),
        copy_layout: &wgpu::BindGroupLayout,
    ) {
        let Some(targets) = graph.targets.as_ref() else {
            return;
        };
        let view = |slot: Slot| match slot {
            Slot::Scene => targets.scene.as_ref(),
            Slot::Scene2 => targets.others[0].as_ref(),
            Slot::Shown => targets.others[1].as_ref(),
            Slot::Shown2 => targets.others[2].as_ref(),
        };
        if self.compiled != graph.compiled {
            self.rest();
            self.compiled = graph.compiled;
        }
        // The frame's clock (its own time, never the wall's: a recorded
        // frame repeats), for every pass of it.
        let (first, last) = *self.clock.get_or_insert((now, now));
        self.clock = Some((first, now));
        let frame = Frame {
            time: now.duration_since(first).as_secs_f32(),
            delta: now.duration_since(last).as_secs_f32(),
            index: index as u32,
            pad: 0,
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
        // Those the device took (each pipeline it may need made first): a
        // pass it refused is left out, the others still in turn.
        let mut ours: Vec<&AppPass> = Vec::new();
        for pass in passes.iter().filter(|p| p.stage == stage) {
            let mut ok = self.prepare(device, queue, pass, (super::SCENE, false));
            if stage == PostStage::Display {
                ok &= self.prepare(device, queue, pass, (format, encode));
            }
            if ok {
                ours.push(pass);
            }
        }
        let (a, b) = match stage {
            PostStage::Hdr => (Slot::Scene, Slot::Scene2),
            PostStage::Display => (Slot::Shown, Slot::Shown2),
        };
        for (at, pass) in ours.iter().enumerate() {
            let (from, to) = if at % 2 == 0 { (a, b) } else { (b, a) };
            let last = at + 1 == ours.len();
            // A display stage's last pass writes the target.
            let (into, format, encode) = match (stage, last) {
                (PostStage::Display, true) => (Some(target), format, encode),
                _ => (view(to), super::SCENE, false),
            };
            let (Some(input), Some(into)) = (view(from), into) else {
                return;
            };
            let key = (pass.id, from as usize);
            if !self.binds.contains_key(&key) {
                let made = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D post, app"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.frame.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.params[&pass.id].as_entire_binding(),
                        },
                    ],
                });
                self.binds.insert(key, made);
            }
            draw(
                encoder,
                &self.pipelines[&(pass.id, format, encode)],
                &self.binds[&key],
                into,
            );
        }
        // An odd count of HDR passes ended in the scene's other half: back
        // into the scene, where bloom and the last pass read it.
        if stage == PostStage::Hdr
            && ours.len() % 2 == 1
            && let (Some(from), Some(into)) = (view(Slot::Scene2), view(Slot::Scene))
        {
            let sampler = &self.sampler;
            let source = self.copy_bind.get_or_insert_with(|| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D post, copy"),
                    layout: copy_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(from),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                })
            });
            draw(encoder, copy, source, into);
        }
    }

    /// `pass`'s module, pipeline into `format` and parameters made (once
    /// each), its parameters written. Whether it can run: false where the
    /// device refused its shader or pipeline (naga passed it; a backend's
    /// translation may not), noted in `failed` and not tried again.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &AppPass,
        (format, encode): (wgpu::TextureFormat, bool),
    ) -> bool {
        if self.failed.contains_key(&pass.id) {
            return false;
        }
        let key = (pass.id, format, encode);
        if !self.pipelines.contains_key(&key) {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let made = self.made(device, pass, (format, encode));
            let error = now(scope.pop()).map(|e| e.to_string());
            #[cfg(test)]
            let error = error.or_else(|| tests::refused(pass.id));
            if let Some(error) = error {
                self.failed.insert(pass.id, error);
                self.modules.remove(&pass.id);
                return false;
            }
            self.pipelines.insert(key, made);
        }
        let bytes: Vec<f32> = pass.values.iter().flatten().copied().collect();
        let size = (bytes.len() as u64 * 4).max(16);
        let buffer = self.params.entry(pass.id).or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D post, app params"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        if !bytes.is_empty() {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&bytes));
        }
        true
    }

    /// `pass`'s module (made once) and its pipeline into `format`.
    fn made(
        &mut self,
        device: &wgpu::Device,
        pass: &AppPass,
        (format, encode): (wgpu::TextureFormat, bool),
    ) -> wgpu::RenderPipeline {
        let module = self.modules.entry(pass.id).or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D post, app"),
                source: wgpu::ShaderSource::Wgsl(pass.wgsl.to_string().into()),
            })
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D post, app"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("zgx_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("zgx_fragment"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("ZGX_ENCODE", f64::from(u8::from(encode)))],
                    ..Default::default()
                },
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Lets go of what passes no longer in `passes` had.
    pub(super) fn retain(&mut self, passes: &[AppPass]) {
        let live = |id: &PostPassId| passes.iter().any(|p| p.id == *id);
        self.modules.retain(|id, _| live(id));
        self.pipelines.retain(|(id, ..), _| live(id));
        self.params.retain(|id, _| live(id));
        self.failed.retain(|id, _| live(id));
        self.binds.retain(|(id, _), _| live(id));
    }
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    into: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D post, app"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: into,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

/// `future`, which resolves at once (wgpu's error scopes on native).
fn now<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{Camera, Geometry, Material, Object, PostParam, PostPass, ToneMapping, World};
    use std::cell::{Cell, RefCell};
    use zgx::{compose::prelude::*, widgets::Ui};
    use zgx_gpu::GpuRenderer;

    thread_local! {
        /// The pass a test has the device refuse, and how often it was
        /// tried.
        static REFUSE: Cell<Option<PostPassId>> = const { Cell::new(None) };
        static TRIED: Cell<u32> = const { Cell::new(0) };
        /// The HDR passes the last frame's chain counted.
        pub(in crate::render) static COUNTED: RefCell<Option<u8>> = const { RefCell::new(None) };
    }
    /// A refusal, as a backend's would come, for the pass a test names.
    pub(super) fn refused(id: PostPassId) -> Option<String> {
        (REFUSE.with(Cell::get) == Some(id)).then(|| {
            TRIED.with(|t| t.set(t.get() + 1));
            "refused for the test".into()
        })
    }

    const SCALE: &str = "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * params.k;
}
";

    #[test]
    fn a_pass_the_device_refuses_is_left_out_and_not_tried_again() {
        const SIZE: u32 = 32;
        let world = World::new();
        let grey = Material {
            color: [0.2, 0.2, 0.2, 1.],
            unlit: true,
            ..Default::default()
        };
        world.add(Object::mesh(Geometry::cuboid(20., 20., 0.1), grey));
        world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
        world.set_tone_mapping(ToneMapping::Linear);
        let mut gpu = match GpuRenderer::new(SIZE, SIZE) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return,
        };
        let mut ui = Ui::new(SIZE as f32, SIZE as f32);
        ui.mount(gpu_view("World", world.view()).size(SIZE as f32, SIZE as f32));
        let mut centre = |step: f32| {
            world.set_camera(Camera::looking_at([0., 0., 4. + step * 1e-4], [0., 0., 0.]));
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            let pixels = gpu.readback().unwrap();
            pixels[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize]
        };
        let scale = || PostPass::new(SCALE).param("k", PostParam::Float);
        let double = world.add_post(scale()).unwrap();
        world.set_post_param(double, "k", 2.).unwrap();
        let doubled = centre(0.);
        // A second, halving, refused: left out, the first's light alone.
        let halve = world.add_post(scale()).unwrap();
        world.set_post_param(halve, "k", 0.5).unwrap();
        REFUSE.with(|r| r.set(Some(halve)));
        for step in 1..4 {
            assert_eq!(centre(step as f32), doubled, "frame {step}");
        }
        assert_eq!(TRIED.with(Cell::get), 1, "tried once");
        assert_eq!(COUNTED.with(|c| *c.borrow()), Some(1), "counted out");
    }
}
