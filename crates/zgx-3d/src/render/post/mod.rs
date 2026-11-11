//! The post chain's passes (design-3d-aaa.md, 6.1): bloom's steps, and the
//! last pass, which mixes bloom in, exposes the scene, maps its tones and
//! writes the view's target. Made with a world's first frame with one on.
mod apps;
pub(in crate::render) use apps::Apps;
/// For the tests: the HDR passes the frame's chain counted.
#[cfg(test)]
pub(super) fn counted(hdr: u8) {
    apps::tests::COUNTED.with(|c| *c.borrow_mut() = Some(hdr));
}
/// For the tests: whether the frame's chain takes the device to run no
/// compute.
#[cfg(test)]
pub(super) fn without_compute() -> bool {
    apps::tests::WITHOUT_COMPUTE.with(std::cell::Cell::get)
}
mod exposure;
mod taa;
pub(super) use taa::{At, jitter, jittered};

use super::graph::{Graph, SCENE, bloom_format};
use zgx_gpu::wgpu;

/// The last pass's numbers (the shader's `Look`).
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Look {
    /// The exposure's factor, the tone mapping (0 neutral, 1 ACES, 2 none),
    /// whether to encode sRGB itself.
    pub(super) params: [f32; 4],
    /// What shows where no mesh drew, premultiplied.
    pub(super) background: [f32; 4],
    /// Bloom's intensity, 1 over its steps, and whether it has a second
    /// (whose tent the last pass adds to the first).
    pub(super) bloom: [f32; 4],
    /// The vignette's, grain's and chromatic aberration's intensities, and
    /// the frame's index.
    pub(super) lens: [f32; 4],
    /// Grading: white balance's factors and saturation, lift and the
    /// table's size, 1 over gamma, gain.
    pub(super) balance: [f32; 4],
    pub(super) lift: [f32; 4],
    pub(super) gamma: [f32; 4],
    pub(super) gain: [f32; 4],
}

/// What the last pass has compiled in: bloom, vignette, grain, chromatic
/// aberration, a bit each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Parts(pub(super) u8);
impl Parts {
    pub(super) const BLOOM: u8 = 1;
    pub(super) const VIGNETTE: u8 = 2;
    pub(super) const GRAIN: u8 = 4;
    pub(super) const ABERRATION: u8 = 8;
    pub(super) const GRADE: u8 = 16;
    pub(super) const TABLE: u8 = 32;
    /// It resolves the meshes' samples itself (`last_samples`): nothing
    /// before it reads the scene.
    pub(super) const RESOLVES: u8 = 64;
    fn constants(self) -> [(&'static str, f64); 6] {
        let on = |bit: u8| f64::from(u8::from(self.0 & bit != 0));
        [
            ("BLOOM", on(Self::BLOOM)),
            ("VIGNETTE", on(Self::VIGNETTE)),
            ("GRAIN", on(Self::GRAIN)),
            ("ABERRATION", on(Self::ABERRATION)),
            ("GRADE", on(Self::GRADE)),
            ("TABLE", on(Self::TABLE)),
        ]
    }
}

/// The bindings of a compile's targets.
struct Binds {
    compiled: u64,
    /// The bytes of the targets they hold.
    bytes: u64,
    /// The scene, read by bloom's first step (none where the last pass
    /// resolves the samples itself).
    scene: Option<wgpu::BindGroup>,
    /// Each of bloom's steps, read by the next step down and the one up.
    steps: Vec<wgpu::BindGroup>,
    /// The scene and bloom's first step, read by the last pass.
    last: wgpu::BindGroup,
    /// The meshes' samples, where the chain resolves them.
    samples: Option<wgpu::BindGroup>,
}

/// The chain's pipelines, layouts and the bindings of the graph's targets.
pub(super) struct Chain {
    shader: wgpu::ShaderModule,
    step_layout: wgpu::BindGroupLayout,
    last_layout: wgpu::BindGroupLayout,
    /// The last pass's, reading the meshes' samples in place of the scene.
    last_samples_layout: wgpu::BindGroupLayout,
    last_samples_pipeline_layout: wgpu::PipelineLayout,
    samples_layout: wgpu::BindGroupLayout,
    /// The meshes' samples resolved into the scene.
    resolve: wgpu::RenderPipeline,
    last_pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    /// Bloom's first step down, the others, and each step up.
    first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    /// The last pass by the target's format and what it has compiled in.
    lasts: Vec<((wgpu::TextureFormat, Parts), wgpu::RenderPipeline)>,
    look: wgpu::Buffer,
    /// What the last pass reads for bloom without it.
    stand_in: wgpu::TextureView,
    /// The grading table on the device, by its id.
    table: Option<(u64, wgpu::TextureView)>,
    /// A copy back into the scene after an odd count of the app's HDR
    /// passes (`Apps`, kept by the renderer).
    copy: wgpu::RenderPipeline,
    /// The binding of what the meshes drew under temporal antialiasing, for
    /// a frame it doesn't blend, by the compile it was made for.
    fresh: Option<(u64, wgpu::BindGroup)>,
    /// Automatic exposure's state (kept between frames, zero until it runs:
    /// unused), and its passes, made with the first frame adapting.
    state: wgpu::Buffer,
    exposure: Option<exposure::Exposure>,
    /// Temporal antialiasing's history and pass, made with its first frame.
    taa: Option<taa::Taa>,
    binds: Option<Binds>,
}

impl Chain {
    #[cold]
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D bloom step"),
            entries: &[texture(0), sampler_entry(1)],
        });
        let last_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D post, last"),
            entries: &[
                texture(0),
                texture(1),
                texture(4),
                texture(5),
                sampler_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // As the last pass's, the meshes' samples at 0.
        let last_samples_layout = {
            let mut entries = vec![wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: true,
                },
                count: None,
            }];
            entries.extend([texture(1), texture(4), texture(5), sampler_entry(2)]);
            for binding in [3, 6] {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                });
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("3D post, last of the samples"),
                entries: &entries,
            })
        };
        let last_samples_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D post, last of the samples"),
                bind_group_layouts: &[Some(&last_samples_layout)],
                immediate_size: 0,
            });
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
        let samples_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D post, samples"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                },
                uniform(3),
                uniform(6),
            ],
        });
        let resolve = pipeline(
            device,
            (&shader, "resolve", &[]),
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D post, samples"),
                bind_group_layouts: &[Some(&samples_layout)],
                immediate_size: 0,
            }),
            SCENE.into(),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3D post"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let step_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D bloom step"),
            bind_group_layouts: &[Some(&step_layout)],
            immediate_size: 0,
        });
        let step = |entry: &str, blend: Option<wgpu::BlendState>| {
            pipeline(
                device,
                (&shader, entry, &[]),
                &step_pipeline_layout,
                wgpu::ColorTargetState {
                    format: bloom_format(device),
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                },
            )
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let (first, down) = (step("first", None), step("down", None));
        let copy = pipeline(
            device,
            (&shader, "copy", &[]),
            &step_pipeline_layout,
            SCENE.into(),
        );
        let up = step(
            "up",
            Some(wgpu::BlendState {
                color: add,
                alpha: add,
            }),
        );
        let last_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D post, last"),
            bind_group_layouts: &[Some(&last_layout)],
            immediate_size: 0,
        });
        let look = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D post, look"),
            size: std::mem::size_of::<Look>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stand_in = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("3D bloom stand-in"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SCENE,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            shader,
            step_layout,
            last_layout,
            last_samples_layout,
            last_samples_pipeline_layout,
            samples_layout,
            resolve,
            last_pipeline_layout,
            sampler,
            first,
            down,
            up,
            lasts: Vec::new(),
            look,
            stand_in,
            table: None,
            copy,
            fresh: None,
            state: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D exposure state"),
                size: exposure::STATE,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            exposure: None,
            taa: None,
            binds: None,
        }
    }

    /// Lets go of the bindings of the graph's targets (they hold them).
    pub(super) fn rest(&mut self) {
        self.binds = None;
        self.fresh = None;
        if let Some(exposure) = &mut self.exposure {
            exposure.rest();
        }
        if let Some(taa) = &mut self.taa {
            taa.rest();
        }
    }

    /// Temporal antialiasing's blend (`taa::Taa::run`): whether it has
    /// frames still to gather since the world last changed.
    pub(super) fn taa(
        &mut self,
        device: (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        at: &taa::At,
        look: (f32, f32),
    ) -> bool {
        let taa = self.taa.get_or_insert_with(|| taa::Taa::new(device.0));
        let (gathering, blended) = taa.run(device, encoder, graph, at, look);
        // Not blended (a size not yet stood): the scene is what the meshes
        // drew.
        if !blended
            && let Some(targets) = graph.targets.as_ref()
            && let (Some(fresh), Some(scene)) = (targets.others[3].as_ref(), targets.scene.as_ref())
        {
            if self
                .fresh
                .as_ref()
                .is_none_or(|(c, _)| *c != graph.compiled)
            {
                let bind = device.0.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D TAA, as drawn"),
                    layout: &self.step_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(fresh),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                self.fresh = Some((graph.compiled, bind));
            }
            if let Some((_, bind)) = &self.fresh {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("3D TAA, as drawn"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: scene,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.copy);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        gathering
    }
    /// Temporal antialiasing's history let go of (it's off).
    pub(super) fn drop_taa(&mut self) {
        self.taa = None;
        self.fresh = None;
    }
    /// The bytes temporal antialiasing's history holds.
    pub(super) fn history_bytes(&self) -> u64 {
        self.taa.as_ref().map_or(0, taa::Taa::bytes)
    }

    /// Automatic exposure's passes (`exposure::Exposure::run`): whether it
    /// had settled, where that came back.
    pub(super) fn expose(
        &mut self,
        device: (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        settings: &crate::world::AutoExposure,
        time: std::time::Instant,
    ) -> Option<bool> {
        let exposure = self
            .exposure
            .get_or_insert_with(|| exposure::Exposure::new(device.0));
        exposure.run(device, encoder, graph, &self.state, settings, time)
    }

    /// The app's passes of `stage` (`apps::Apps::run`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apps(
        &mut self,
        apps: &mut Apps,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        passes: &[crate::world::AppPass],
        stage: crate::world::PostStage,
        into: (&wgpu::TextureView, wgpu::TextureFormat, bool),
        tick: super::Tick,
    ) {
        let copy = (tick, &self.copy);
        apps.run(
            (device, queue),
            encoder,
            graph,
            passes,
            stage,
            into,
            copy,
            &self.step_layout,
        );
    }
    /// The bytes of the targets its bindings hold: at most those of the
    /// compile they were made for.
    pub(super) fn held(&self) -> u64 {
        self.binds.as_ref().map_or(0, |b| b.bytes)
    }

    /// The bindings of `graph`'s targets (`binds`), made again as it
    /// compiles.
    fn made(&mut self, device: &wgpu::Device, graph: &Graph) -> Option<()> {
        let targets = graph.targets.as_ref()?;
        if self
            .binds
            .as_ref()
            .is_none_or(|b| b.compiled != graph.compiled)
        {
            // The scene, or where the last pass resolves the samples
            // itself, the samples.
            let scene = targets.scene.as_ref();
            let drawn = match scene {
                Some(scene) => (scene, &self.last_layout),
                None => (
                    targets.colour.as_ref().filter(|_| targets.stored)?,
                    &self.last_samples_layout,
                ),
            };
            let step = |view: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D bloom step"),
                    layout: &self.step_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            };
            // Bloom's first step, and the second (its tent up to the first
            // is the last pass's own).
            let glow = targets.bloom.first().unwrap_or(&self.stand_in);
            let wider = targets.bloom.get(1).unwrap_or(&self.stand_in);
            let last = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D post, last"),
                layout: drawn.1,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(drawn.0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(glow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.look.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(wider),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(
                            self.table.as_ref().map_or(&self.stand_in, |(_, view)| view),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: self.state.as_entire_binding(),
                    },
                ],
            });
            let samples = targets.colour.as_ref();
            let samples = samples.filter(|_| targets.stored && scene.is_some());
            let samples = samples.map(|colour| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D post, samples"),
                    layout: &self.samples_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(colour),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.look.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: self.state.as_entire_binding(),
                        },
                    ],
                })
            });
            self.binds = Some(Binds {
                compiled: graph.compiled,
                samples,
                bytes: graph.bytes(),
                scene: scene.map(step),
                steps: targets.bloom.iter().map(step).collect(),
                last,
            });
        }
        Some(())
    }

    /// The meshes' samples resolved into the scene (`Pass::Resolve`).
    pub(super) fn resolve(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
    ) {
        if self.made(device, graph).is_none() {
            return;
        }
        let (Some(binds), Some(scene)) = (
            &self.binds,
            graph.targets.as_ref().and_then(|t| t.scene.as_ref()),
        ) else {
            return;
        };
        let Some(samples) = &binds.samples else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D post, resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.resolve);
        pass.set_bind_group(0, samples, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Bloom's steps: each made from the one before (the scene for the
    /// first), then each from the smaller one after added to it.
    pub(super) fn bloom(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
    ) {
        let Some(steps) = graph.targets.as_ref().map(|t| &t.bloom) else {
            return;
        };
        if self.made(device, graph).is_none() {
            return;
        }
        let Some(binds) = &self.binds else {
            return;
        };
        let draw = |encoder: &mut wgpu::CommandEncoder,
                    pipeline: &wgpu::RenderPipeline,
                    source: &wgpu::BindGroup,
                    target: &wgpu::TextureView,
                    load: wgpu::LoadOp<wgpu::Color>| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D bloom step"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, source, &[]);
            pass.draw(0..3, 0..1);
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        for (at, target) in steps.iter().enumerate() {
            match at {
                0 => {
                    let Some(scene) = &binds.scene else {
                        return;
                    };
                    draw(encoder, &self.first, scene, target, clear);
                }
                _ => draw(encoder, &self.down, &binds.steps[at - 1], target, clear),
            }
        }
        // Up to the second: the last pass adds the second's tent to the
        // first as it reads them.
        for at in (1..steps.len().saturating_sub(1)).rev() {
            let load = wgpu::LoadOp::Load;
            draw(encoder, &self.up, &binds.steps[at + 1], &steps[at], load);
        }
    }

    /// The last pass: the scene with `parts`, exposed and mapped as `look`
    /// says, into `target` (of `format`).
    pub(super) fn last(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        (target, format): (&wgpu::TextureView, wgpu::TextureFormat),
        (look, parts): (Look, Parts),
        table: Option<&crate::Lut>,
    ) {
        // The grading table uploaded as it changes (its binding with it).
        if let Some(lut) = table
            && self.table.as_ref().is_none_or(|(id, _)| *id != lut.id())
        {
            let n = lut.size();
            let texture = zgx_gpu::viewport::texture_with_levels(
                device,
                queue,
                (n * n, n),
                wgpu::TextureFormat::Rgba8Unorm,
                &[lut.strip()],
                "3D grading table",
            );
            self.table = Some((lut.id(), texture.create_view(&Default::default())));
            self.binds = None;
        }
        // Resolving the samples itself where nothing before needs the scene.
        let resolves = graph.targets.as_ref().is_some_and(|t| t.scene.is_none());
        let parts = Parts(parts.0 | if resolves { Parts::RESOLVES } else { 0 });
        let key = (format, parts);
        if !self.lasts.iter().any(|(k, _)| *k == key) {
            let (entry, layout) = match resolves {
                true => ("last_samples", &self.last_samples_pipeline_layout),
                false => ("last", &self.last_pipeline_layout),
            };
            let made = pipeline(
                device,
                (&self.shader, entry, &parts.constants()),
                layout,
                format.into(),
            );
            self.lasts.push((key, made));
        }
        queue.write_buffer(&self.look, 0, bytemuck::bytes_of(&look));
        if self.made(device, graph).is_none() {
            return;
        }
        let Some(binds) = &self.binds else {
            return;
        };
        let pipeline = &self.lasts.iter().find(|(k, _)| *k == key);
        let Some((_, pipeline)) = pipeline else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D post, last"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_bind_group(0, &binds.last, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A full-screen pass's pipeline: `entry` of `shader` with `constants`.
fn pipeline(
    device: &wgpu::Device,
    (shader, entry, constants): (&wgpu::ShaderModule, &str, &[(&str, f64)]),
    layout: &wgpu::PipelineLayout,
    target: wgpu::ColorTargetState,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D post"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            targets: &[Some(target)],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}
