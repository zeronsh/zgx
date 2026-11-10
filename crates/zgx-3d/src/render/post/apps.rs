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
        let ours: Vec<&AppPass> = passes.iter().filter(|p| p.stage == stage).collect();
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
            self.prepare(device, queue, pass, (format, encode));
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
    /// each), its parameters written.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &AppPass,
        (format, encode): (wgpu::TextureFormat, bool),
    ) {
        let module = self.modules.entry(pass.id).or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D post, app"),
                source: wgpu::ShaderSource::Wgsl(pass.wgsl.to_string().into()),
            })
        });
        let layout = &self.pipeline_layout;
        self.pipelines
            .entry((pass.id, format, encode))
            .or_insert_with(|| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("3D post, app"),
                    layout: Some(layout),
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
            });
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
    }

    /// Lets go of what passes no longer in `passes` had.
    pub(super) fn retain(&mut self, passes: &[AppPass]) {
        let live = |id: &PostPassId| passes.iter().any(|p| p.id == *id);
        self.modules.retain(|id, _| live(id));
        self.pipelines.retain(|(id, ..), _| live(id));
        self.params.retain(|id, _| live(id));
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
