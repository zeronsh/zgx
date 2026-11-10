//! What is behind meshes light passes through: the frame's other meshes,
//! drawn first into a texture of their own with every smaller level, for
//! rough glass to read blurred.
use super::{
    Gpu, Run,
    draw::{Bound, draw_run},
};
use zgx_gpu::wgpu;
/// The frame's meshes but those light passes through (and translucent
/// ones), drawn into what is behind them, then its smaller levels. Apart
/// from `render`: a second copy of its loop there outlined what it calls.
#[cold]
#[inline(never)]
pub(super) fn draw_behind(
    encoder: &mut wgpu::CommandEncoder,
    gpu: &Gpu,
    runs: &[Run],
    frame_bind: &wgpu::BindGroup,
    clear: wgpu::Color,
) {
    let (Some(behind), Some((instances_buffer, _)), Some((_, colour, depth_view))) =
        (&gpu.behind, &gpu.instances, &gpu.targets)
    else {
        return;
    };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D behind"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: colour.as_ref().unwrap_or(&behind.levels[0]),
            depth_slice: None,
            resolve_target: colour.as_ref().map(|_| &behind.levels[0]),
            ops: wgpu::Operations {
                // What shows where nothing is: the frame's background.
                load: wgpu::LoadOp::Clear(clear),
                store: if colour.is_some() {
                    wgpu::StoreOp::Discard
                } else {
                    wgpu::StoreOp::Store
                },
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    pass.set_bind_group(0, frame_bind, &[]);
    pass.set_vertex_buffer(1, instances_buffer.slice(..));
    let mut bound = Bound::default();
    for run in runs
        .iter()
        .filter(|run| !run.glass && !run.variant.transmitting())
    {
        draw_run(&mut pass, gpu, run, &mut bound);
    }
    drop(pass);
    behind.shrink(encoder);
}

/// What is behind meshes light passes through: the frame's others, drawn
/// at its size into a texture of every level down to one texel.
pub(super) struct Behind {
    pub(super) size: (u32, u32),
    /// Each level's view, to draw into, and to make the next from.
    pub(super) levels: Vec<wgpu::TextureView>,
    /// All levels, to read.
    pub(super) whole: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
    /// Making each level from the one before: its pipeline, and each
    /// level's binding of the one before.
    pub(super) shrinking: wgpu::RenderPipeline,
    pub(super) sources: Vec<wgpu::BindGroup>,
}
impl Behind {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let levels = 32 - width.max(height).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3D behind"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level_views: Vec<wgpu::TextureView> = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3D behind"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D behind, shrinking"),
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
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D behind, shrinking"),
            source: wgpu::ShaderSource::Wgsl(SHRINK.into()),
        });
        let shrinking = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D behind, shrinking"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("3D behind, shrinking"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sources = level_views[..level_views.len() - 1]
            .iter()
            .map(|source| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D behind, a level"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                })
            })
            .collect();
        Self {
            size,
            whole: texture.create_view(&Default::default()),
            levels: level_views,
            sampler,
            shrinking,
            sources,
        }
    }
    /// Each level made from the one before.
    pub(super) fn shrink(&self, encoder: &mut wgpu::CommandEncoder) {
        for (target, source) in self.levels[1..].iter().zip(&self.sources) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D behind, a level"),
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
            pass.set_pipeline(&self.shrinking);
            pass.set_bind_group(0, source, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
/// A level made from the one before: each texel the mean of the four
/// beneath it (one linear sample between them).
pub(super) const SHRINK: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv);
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, linear, in.uv, 0.0);
}
"#;
