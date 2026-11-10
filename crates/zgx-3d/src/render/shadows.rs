//! The sun's shadow map: its view of the opaque meshes, its pipelines and
//! its texture.
use super::{
    DEPTH, Gpu, Run, Variant,
    materials::{Extent, layout_of, make_layout},
    pipelines::vertex_buffers,
    shader::wgsl,
};
use crate::math::{Mat4, Vec3};
use zgx_gpu::wgpu;
#[cfg(feature = "lighting")]
pub(super) mod cascades;
/// The shadow map's side, in texels.
pub(super) const SHADOW_SIZE: u32 = 2048;

/// What the shadow map holds: the sun's view, each opaque mesh's geometry,
/// place and alpha cutoff, skinned ones' joint matrices, morphing ones'
/// weights, and the maps' key of those casting cut out (`map_set`: the base
/// colour map's alpha cuts, as placed), in turn (plain casters pay none). Every input the shadow pass reads
/// of a mesh: what isn't here can't change what the map holds.
pub(super) type Casting = (
    Mat4,
    Vec<(u64, Mat4, u32)>,
    Vec<Mat4>,
    Vec<[f32; 2]>,
    Vec<u64>,
);

/// A shadow pipeline's bits (`shadow_key`).
pub(super) const CUT_OUT: u8 = 4;
pub(super) const PLACED: u8 = 8;
/// What sets `variant`'s shadow pipeline apart, as one number (looked up a
/// run a frame): skinned, morphing, cut out (`CUT_OUT`) and, cut out,
/// placed (`PLACED`) and layered (a coat or specular), its maps bound as its
/// mesh's. A mesh light
/// passes through casts its whole shadow (its binding isn't a cut-out
/// shadow's).
#[inline(always)]
pub(super) fn shadow_key(variant: Variant) -> u8 {
    #[cfg(not(feature = "shaders"))]
    let masked = variant.masked() && !variant.transmitting();
    // A custom material's mesh never casts by a stock cut-out pipeline: its
    // binding is its shader's (`custom::draw_shadow`).
    #[cfg(feature = "shaders")]
    let masked =
        variant.0 & (Variant::MASKED | Variant::TRANSMITTING | Variant::CUSTOM) == Variant::MASKED;
    let bit = |on: bool, bit: u8| if on { bit } else { 0 };
    let key = bit(variant.skinned(), 1)
        | bit(variant.morphing(), 2)
        | bit(masked, CUT_OUT)
        | bit(masked && variant.placed(), PLACED)
        | bit(masked && (variant.coated() || variant.specular()), 16);
    // A custom material's: none of these (`custom::draw_shadow`).
    #[cfg(feature = "shaders")]
    let key = key | bit(variant.custom(), CUSTOM);
    key
}
/// A custom material's shadow key's bit: no stock pipeline has it.
#[cfg(feature = "shaders")]
pub(super) const CUSTOM: u8 = 32;
/// A skinned, morphing or cut-out shadow's pipeline for `variant`'s meshes.
/// Made once, rarely: out of the frame's code.
#[cold]
#[inline(never)]
pub(super) fn make_shadow_pipeline(gpu: &mut Gpu, device: &wgpu::Device, variant: Variant) {
    // (A custom material's are its shader's: `custom::prepare`.)
    #[cfg(feature = "shaders")]
    if variant.custom() {
        return;
    }
    let key = shadow_key(variant);
    let pipeline = caster_pipeline(gpu, device, variant, DEPTH);
    gpu.shadow_variants.insert(key, pipeline);
}
/// The pipeline drawing `variant`'s meshes' depth from the sun into a map
/// of `format`: today's map's or a cascade's.
#[cold]
#[inline(never)]
pub(super) fn caster_pipeline(
    gpu: &mut Gpu,
    device: &wgpu::Device,
    variant: Variant,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let (skinned, morphing) = (variant.skinned(), variant.morphing());
    // A mesh light passes through casts its whole shadow (as, here, does
    // a custom material's: `shadow_key`).
    let masked = variant.masked() && !variant.transmitting();
    #[cfg(feature = "shaders")]
    let masked = masked && !variant.custom();
    let placed = masked && variant.placed();
    let layered = masked && (variant.coated() || variant.specular());
    let variant = Variant::default()
        .with(Variant::TEXTURED, masked)
        .with(Variant::SKINNED, skinned)
        .with(Variant::MORPHING, morphing)
        .with(Variant::MASKED, masked)
        .with(Variant::PLACED, placed);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("3D skinned, morphing or cut-out shadows"),
        source: wgpu::ShaderSource::Wgsl(wgsl(variant, 1).into()),
    });
    // A cut-out shadow binds its mesh's maps, as its mesh does.
    let extent = if layered {
        Some(Extent::Layered)
    } else if placed {
        Some(Extent::Placed)
    } else {
        masked.then_some(Extent::Maps)
    };
    make_layout(gpu, device, extent);
    let group1 = layout_of(gpu, extent);
    let layouts = [
        Some(&gpu.shadow_layout),
        Some(group1),
        Some(&gpu.palette_layout),
        Some(&gpu.morph_layout),
    ];
    let count = match (skinned, morphing) {
        (_, true) => 4,
        (true, false) => 3,
        (false, false) => 2,
    };
    shadow_pipeline_in(device, &shader, &layouts[..count], variant, format)
}

/// The box holding the box `(low, high)` after `matrix`, which has no
/// perspective: what its eight corners would give, each axis's least and
/// greatest added up.
pub(super) fn box_through(matrix: &Mat4, (low, high): (Vec3, Vec3)) -> (Vec3, Vec3) {
    let c = &matrix.columns;
    let (mut from, mut to) = ([c[3][0], c[3][1], c[3][2]], [c[3][0], c[3][1], c[3][2]]);
    for (column, (l, h)) in c
        .iter()
        .zip([(low.x, high.x), (low.y, high.y), (low.z, high.z)])
    {
        for row in 0..3 {
            let (a, b) = (column[row] * l, column[row] * h);
            from[row] += a.min(b);
            to[row] += a.max(b);
        }
    }
    (Vec3::from(from), Vec3::from(to))
}
/// The box holding the opaque meshes of `all`, each as placed (the
/// cascades' casters' where the frame made no boxes).
#[cfg(feature = "lighting")]
#[cold]
pub(super) fn sun_bounds(all: &[crate::world::Drawn]) -> (Vec3, Vec3) {
    let (mut low, mut high) = (Vec3::from([f32::MAX; 3]), Vec3::from([f32::MIN; 3]));
    for d in all.iter().filter(|d| d.material.color[3] >= 1.) {
        let (bounds, matrices) = placed(d);
        for matrix in matrices {
            let (from, to) = box_through(matrix, bounds);
            low = low.min(from);
            high = high.max(to);
        }
    }
    (low, high)
}

/// The sun going `direction` as it sees the opaque meshes of `all`: an
/// orthographic view holding them all, onto the shadow map, and how wide a
/// map texel is in the scene. `None` with nothing to cast a shadow.
pub(super) fn sun_view(all: &[crate::world::Drawn], direction: Vec3) -> Option<(Mat4, f32)> {
    let opaque = || all.iter().filter(|d| d.material.color[3] >= 1.);
    let (mut low, mut high) = (Vec3::from([f32::MAX; 3]), Vec3::from([f32::MIN; 3]));
    for d in opaque() {
        let (bounds, matrices) = placed(d);
        for matrix in matrices {
            let (from, to) = box_through(matrix, bounds);
            low = low.min(from);
            high = high.max(to);
        }
    }
    fit(all, (low, high), direction)
}
/// As `sun_view`, the opaque meshes held by `(low, high)` (their boxes'
/// union, `cull::world_box`: the same box `sun_view` finds).
pub(super) fn sun_view_within(
    all: &[crate::world::Drawn],
    (low, high): (Vec3, Vec3),
    direction: Vec3,
) -> Option<(Mat4, f32)> {
    fit(all, (low, high), direction)
}
/// A mesh's box, and what places it. A skinned mesh's is placed by each of
/// its joints: about where its pose can take it.
fn placed<'a>(d: &'a crate::world::Drawn) -> ((Vec3, Vec3), &'a [Mat4]) {
    let bounds = match &d.morph {
        Some(active) => d.geometry.morphed_bounds(active),
        None => d.geometry.bounds(),
    };
    let matrices = d.palette;
    (bounds, matrices.unwrap_or(std::slice::from_ref(&d.matrix)))
}
/// The sun's view holding the opaque meshes of `all`, which `(low, high)`
/// holds (`sun_view`).
// Inline: `sun_view` compiled as one function, as it was before culling.
#[inline(always)]
fn fit(
    all: &[crate::world::Drawn],
    (low, high): (Vec3, Vec3),
    direction: Vec3,
) -> Option<(Mat4, f32)> {
    let opaque = || all.iter().filter(|d| d.material.color[3] >= 1.);
    let direction = direction.normalized();
    if !(low.is_finite() && high.is_finite()) || direction == Vec3::ZERO {
        return None;
    }
    let centre = (low + high) * 0.5;
    let radius = ((high - low).length() * 0.5).max(1e-3);
    let view = Mat4::look_at(centre - direction * (radius * 2.), centre, Vec3::Y);
    let (mut near, mut far) = (Vec3::from([f32::MAX; 3]), Vec3::from([f32::MIN; 3]));
    for d in opaque() {
        let (bounds, matrices) = placed(d);
        for matrix in matrices {
            let (from, to) = box_through(&(view * *matrix), bounds);
            near = near.min(from);
            far = far.max(to);
        }
    }
    // A texel's margin about them, so the filter's edge falls outside.
    let margin = (far.x - near.x).max(far.y - near.y) / SHADOW_SIZE as f32 * 2.;
    let (l, r) = (near.x - margin, far.x + margin);
    let (b, t) = (near.y - margin, far.y + margin);
    // View space looks down -z: the nearest is the greatest z.
    let (n, f) = (-far.z - margin, -near.z + margin);
    let projection = Mat4 {
        columns: [
            [2. / (r - l), 0., 0., 0.],
            [0., 2. / (t - b), 0., 0.],
            [0., 0., 1. / (n - f), 0.],
            [-(r + l) / (r - l), -(t + b) / (t - b), n / (n - f), 1.],
        ],
    };
    let texel = (r - l).max(t - b) / SHADOW_SIZE as f32;
    Some((projection * view, texel))
}

/// Depth from the sun with `shader`, bound as `layouts` say; a cut-out
/// `variant`'s by its alpha in a fragment shader (depth alone needs none
/// otherwise).
pub(super) fn shadow_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    variant: Variant,
) -> wgpu::RenderPipeline {
    shadow_pipeline_in(device, shader, layouts, variant, DEPTH)
}
/// `shadow_pipeline`, into a map of `format`.
pub(super) fn shadow_pipeline_in(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    variant: Variant,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D shadows"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D shadows"),
                bind_group_layouts: layouts,
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("shadow_vertex"),
            compilation_options: Default::default(),
            buffers: &vertex_buffers(variant),
        },
        fragment: variant.masked().then(|| wgpu::FragmentState {
            module: shader,
            entry_point: Some("shadow_fragment"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        // Both faces: a plane or an open mesh casts from either side.
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            // Kept off the surfaces they're cast on (acne), more where
            // they slope from the sun.
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.,
                clamp: 0.,
            },
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A depth texture `side` texels square, drawn to and sampled.
pub(super) fn depth_map(device: &wgpu::Device, side: u32, label: &str) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// The shadow pass: opaque meshes' depth as the sun sees them, into `map`,
/// each run's `instances` bound.
pub(super) fn draw_shadows(
    encoder: &mut wgpu::CommandEncoder,
    gpu: &Gpu,
    runs: &[Run],
    map: &wgpu::TextureView,
    instances: &wgpu::Buffer,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D shadows"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: map,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    pass.set_bind_group(0, &gpu.shadow_bind, &[]);
    pass.set_vertex_buffer(1, instances.slice(..));
    // Translucent meshes cast none.
    for run in runs.iter().filter(|run| !run.glass) {
        let buffers = &gpu.geometries[&run.geometry];
        let (skinned, morphing) = (run.variant.skinned(), run.variant.morphing());
        let key = shadow_key(run.variant);
        let (masked, placed) = (key & CUT_OUT != 0, key & PLACED != 0);
        match gpu.shadow_variants.get(&key) {
            Some(pipeline) => {
                pass.set_pipeline(pipeline);
                let maps = gpu.bindings.get(&run.maps).filter(|_| masked);
                pass.set_bind_group(1, maps.map_or(&gpu.empty_bind, |(binding, _)| binding), &[]);
                if let Some((_, _, joints)) = gpu.palettes.as_ref().filter(|_| skinned || morphing)
                {
                    pass.set_bind_group(2, joints, &[]);
                }
                if let Some(skin) = buffers.skin.as_ref().filter(|_| skinned) {
                    pass.set_vertex_buffer(2, skin.slice(..));
                }
                if let Some(uvs) = buffers.second_uvs.as_ref().filter(|_| placed) {
                    pass.set_vertex_buffer(2 + u32::from(skinned), uvs.slice(..));
                }
                if let Some(morphs) = buffers.morphs.as_ref().filter(|_| morphing) {
                    pass.set_bind_group(3, morphs, &[]);
                }
            }
            None => {
                // A custom material's (whose key no stock pipeline has).
                #[cfg(feature = "shaders")]
                if run.variant.custom() && super::custom::draw_shadow(&mut pass, gpu, run) {
                    continue;
                }
                pass.set_pipeline(&gpu.shadow)
            }
        }
        pass.set_vertex_buffer(0, buffers.vertices.slice(..));
        pass.set_index_buffer(buffers.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..buffers.count, 0, run.instances.clone());
    }
}

/// Spans `spans` of `runs` (`cull::shown`) drawn as casters into `pass`,
/// its frame and instances bound, with `pipelines` (by `shadow_key`): a
/// cascade's casters. Translucent runs cast nothing.
#[cfg(feature = "lighting")]
pub(super) fn draw_casters(
    pass: &mut wgpu::RenderPass<'_>,
    gpu: &Gpu,
    runs: &[Run],
    spans: &[(u32, std::ops::Range<u32>)],
    pipelines: &std::collections::HashMap<u8, wgpu::RenderPipeline>,
) {
    let mut bound = super::draw::Bound::default();
    for spans in spans.chunk_by(|a, b| a.0 == b.0) {
        let run = &runs[spans[0].0 as usize];
        if run.glass {
            continue;
        }
        let key = shadow_key(run.variant);
        let Some(pipeline) = pipelines.get(&key) else {
            continue;
        };
        let buffers = &gpu.geometries[&run.geometry];
        let (skinned, morphing) = (run.variant.skinned(), run.variant.morphing());
        let (masked, placed) = (key & CUT_OUT != 0, key & PLACED != 0);
        bound.pipeline(pass, pipeline);
        let maps = gpu.bindings.get(&run.maps).filter(|_| masked);
        bound.group(
            pass,
            1,
            maps.map_or(&gpu.empty_bind, |(binding, _)| binding),
        );
        if let Some((_, _, joints)) = gpu.palettes.as_ref().filter(|_| skinned || morphing) {
            bound.group(pass, 2, joints);
        }
        if let Some(skin) = buffers.skin.as_ref().filter(|_| skinned) {
            bound.vertices(pass, 2, skin);
        }
        if let Some(uvs) = buffers.second_uvs.as_ref().filter(|_| placed) {
            bound.vertices(pass, 2 + u32::from(skinned), uvs);
        }
        if let Some(morphs) = buffers.morphs.as_ref().filter(|_| morphing) {
            bound.group(pass, 3, morphs);
        }
        bound.vertices(pass, 0, &buffers.vertices);
        bound.indices(pass, &buffers.indices);
        for (_, instances) in spans {
            pass.draw_indexed(0..buffers.count, 0, instances.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    #[test]
    fn a_box_through_a_matrix_is_what_its_corners_give() {
        let (low, high) = (crate::vec3(-1., 0.5, -2.), crate::vec3(3., 2., 0.25));
        let turned = crate::Quat::from_euler(0.7, -1.2, 2.1);
        let matrix =
            Mat4::from_transform(crate::vec3(4., -3., 0.5), turned, crate::vec3(2., -1., 0.5));
        let view = Mat4::look_at(crate::vec3(3., 9., -6.), Vec3::ZERO, Vec3::Y);
        for matrix in [Mat4::IDENTITY, matrix, view * matrix] {
            let (mut from, mut to) = (Vec3::from([f32::MAX; 3]), Vec3::from([f32::MIN; 3]));
            for i in 0..8 {
                let p = matrix.project(crate::vec3(
                    if i & 1 == 0 { low.x } else { high.x },
                    if i & 2 == 0 { low.y } else { high.y },
                    if i & 4 == 0 { low.z } else { high.z },
                ));
                from = from.min(p);
                to = to.max(p);
            }
            let (a, b) = box_through(&matrix, (low, high));
            for (got, want) in [(a, from), (b, to)] {
                assert!((got - want).length() < 1e-4, "{got:?} {want:?}");
            }
        }
    }
}
