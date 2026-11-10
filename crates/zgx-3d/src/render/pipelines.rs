//! The meshes' pipelines: their vertex buffers and states, made once a
//! variant.
use super::{
    DEPTH, Gpu, Instance, Variant,
    materials::{Extent, layout_of, make_layout},
    shader::wgsl,
};
use zgx_gpu::wgpu;
/// `variant`'s pipelines, opaque and translucent. Made once, rarely: out
/// of the frame's code.
#[cold]
#[inline(never)]
pub(super) fn make_main_pipelines(gpu: &mut Gpu, device: &wgpu::Device, variant: Variant) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("3D mesh variant"),
        source: wgpu::ShaderSource::Wgsl(wgsl(variant, gpu.samples).into()),
    });
    let extent = if variant.transmitting() {
        Some(Extent::Glass)
    } else if variant.coated() || variant.specular() {
        Some(Extent::Layered)
    } else if variant.placed() {
        Some(Extent::Placed)
    } else {
        variant.textured().then_some(Extent::Maps)
    };
    make_layout(gpu, device, extent);
    let group0 = match &gpu.environment_layout {
        Some((layout, _)) if variant.lit() => layout,
        _ => &gpu.layout,
    };
    let group1 = layout_of(gpu, extent);
    let layouts = [
        Some(group0),
        Some(group1),
        Some(&gpu.palette_layout),
        Some(&gpu.morph_layout),
    ];
    let count = match (variant.skinned(), variant.morphing()) {
        (_, true) => 4,
        (true, false) => 3,
        (false, false) => 2,
    };
    let pipelines = main_pipelines(
        device,
        (gpu.format, gpu.samples),
        &shader,
        &layouts[..count],
        variant,
    );
    gpu.variants.insert(variant, pipelines);
}

pub(super) const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
pub(super) const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
    7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4
];
pub(super) const SKIN_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![12 => Uint16x4, 13 => Float32x4];
pub(super) const SECOND_UV_ATTRIBUTES: [wgpu::VertexAttribute; 1] =
    wgpu::vertex_attr_array![14 => Float32x2];
pub(super) const COLOR_ATTRIBUTES: [wgpu::VertexAttribute; 1] =
    wgpu::vertex_attr_array![15 => Float32x4];

/// The vertex buffers a pipeline reads: vertices, instances, a skinned
/// one's joints and weights, and a placed one's second coordinates.
pub(super) fn vertex_buffers(variant: Variant) -> Vec<wgpu::VertexBufferLayout<'static>> {
    let mut buffers = vec![
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::geometry::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        },
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRIBUTES,
        },
    ];
    if variant.skinned() {
        buffers.push(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::geometry::Influence>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SKIN_ATTRIBUTES,
        });
    }
    if variant.placed() {
        buffers.push(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 2]>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SECOND_UV_ATTRIBUTES,
        });
    }
    if variant.colored() {
        buffers.push(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 4]>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &COLOR_ATTRIBUTES,
        });
    }
    buffers
}

/// The opaque and translucent meshes' pipelines with `shader`, bound as
/// `layouts` say, drawing into `format` at `samples` a pixel.
pub(super) fn main_pipelines(
    device: &wgpu::Device,
    (format, samples): (wgpu::TextureFormat, u32),
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    variant: Variant,
) -> [wgpu::RenderPipeline; 2] {
    let buffers = vertex_buffers(variant);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("3D"),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    let pipeline = |translucent: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if translucent {
                "3D translucent"
            } else {
                "3D opaque"
            }),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: translucent.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: (!translucent && !variant.two_sided()).then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(!translucent),
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
    };
    [pipeline(false), pipeline(true)]
}
