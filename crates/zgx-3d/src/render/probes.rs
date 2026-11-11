//! Irradiance probes (`gi` feature, `World::add_probes`): the world's
//! volumes' probes in two atlases (light, `shader/probes.wgsl`; distance),
//! updated where the device traces rays: a budget of probes a frame cast
//! rays into the scene (`raytrace.rs`, `probes_trace.wgsl`), and each ray's
//! light and distance are blended into their maps (`probes_blend.wgsl`)
//! by the round's weight. Each change to the meshes or the light starts
//! the rounds again; after `ROUNDS` with nothing changed the probes have
//! converged and the world settles: it stops drawing.
//!
//! A world without volumes, or on a device without ray queries, makes
//! none of this, and its meshes draw as without the feature.
use super::{frame::FrameUniforms, gpu::Gpu, raytrace::Scene};
use crate::world::{Drawn, Inner, gi::MAX_PROBE_VOLUMES};
use zgx_gpu::wgpu;

/// An irradiance map's side, its border included; a distance map's.
const LIGHT_TILE: u32 = 8;
const DEPTH_TILE: u32 = 16;
/// Probes a row of the atlases.
const COLUMNS: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Rounds (every probe updated once) after a change before the probes are
/// taken as converged. Round `r` blends at `1 / (r + 1)`, `FLOOR` at
/// least: early rounds replace (light follows a change at once), later
/// ones average the noise away and let light bounce on.
pub(crate) const ROUNDS: u32 = 16;
const FLOOR: f32 = 0.2;
/// The world's reason while its probes converge (`Settle::PROBES`).
const SETTLE: u32 = crate::world::Settle::PROBES;

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct VolumeUniform {
    origin: [f32; 4],
    spacing: [f32; 4],
    inverse: [f32; 4],
    counts: [u32; 4],
    lo: [f32; 4],
    hi: [f32; 4],
    fade: [f32; 4],
}
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct SetUniform {
    volumes: [VolumeUniform; MAX_PROBE_VOLUMES],
    count: [u32; 4],
    texel: [f32; 4],
}
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct TraceUniform {
    turn: [[f32; 4]; 4],
    first: u32,
    count: u32,
    rays: u32,
    total: u32,
    far: f32,
    lift: f32,
    /// Point and spot lights in `lights`, and the round (seeding which
    /// light a hit samples where there are many).
    lights: u32,
    seed: u32,
    /// `Gi::poison`, and a zero the GPU divides by where it's on.
    poison: u32,
    zero: f32,
    _pad: [u32; 2],
}
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct AtlasUniform {
    size: [f32; 2],
    tile: f32,
    depth: f32,
}

/// What of the light a frame shades with probes read, but its point and
/// spot lights (`trace_lights`): the sun, the hemisphere and the
/// environment's intensity and turn. None follows the camera.
fn lighting(uniforms: &FrameUniforms) -> [f32; 20] {
    let mut out = [0.; 20];
    let parts: [&[f32; 4]; 5] = [
        &uniforms.sun_direction,
        &uniforms.sun_color,
        &uniforms.sky,
        &uniforms.ground,
        &uniforms.environment,
    ];
    for (i, part) in parts.into_iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(part);
    }
    out
}

/// A point or spot light as a ray's hit is lit by it (`TraceLight` in
/// `probes_trace.wgsl`): as the frame's `points` hold them.
type TraceLight = [[f32; 4]; 3];
/// Every point and spot light of `world`, in the world's order (the
/// frame's `points` are the eight nearest the camera's target, which turn
/// with it), into `out`.
fn trace_lights(world: &Inner, out: &mut Vec<TraceLight>) {
    out.clear();
    world.for_each_light(|light, spot| {
        if let crate::world::Light::Point {
            position,
            color,
            intensity,
            range,
        } = light
        {
            let cone = spot.map_or([0., 0., 0., 1.], |spot| {
                let (inner, outer) = (spot.inner.cos(), spot.outer.cos());
                let scale = 1. / (inner - outer).max(1e-3);
                let d = spot.direction.normalized() * scale;
                [d.x, d.y, d.z, -outer * scale]
            });
            out.push([
                [position.x, position.y, position.z, range.max(1e-3)],
                [color[0], color[1], color[2], intensity],
                cone,
            ]);
        }
    });
}

struct Atlas {
    view: wgpu::TextureView,
    size: [u32; 2],
    uniform: wgpu::Buffer,
}
fn atlas(device: &wgpu::Device, probes: u32, tile: u32, label: &str) -> Atlas {
    let rows = probes.div_ceil(COLUMNS).max(1);
    let size = [COLUMNS * tile, rows * tile];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let uniform = AtlasUniform {
        size: [size[0] as f32, size[1] as f32],
        tile: tile as f32,
        depth: if tile == DEPTH_TILE { 1. } else { 0. },
    };
    let uniform = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    Atlas {
        view: texture.create_view(&Default::default()),
        size,
        uniform,
    }
}

/// How the volumes are laid out on the device, made again as they change
/// (`Gi::layout`).
struct Layout {
    of: u64,
    total: u32,
    rays: u32,
    per_frame: u32,
    far: f32,
    lift: f32,
    light: Atlas,
    depth: Atlas,
    set: wgpu::Buffer,
    /// Every probe's place, in order (as `probe_place` finds it): where
    /// faces are culled from.
    places: Vec<[f32; 3]>,
    /// Written once by a pass clearing them (a blit into a texture opens
    /// driver blocks on Metal; a render pass doesn't).
    cleared: bool,
}

fn layout(device: &wgpu::Device, world: &Inner) -> Layout {
    let gi = &world.gi;
    let mut set = SetUniform::default();
    let mut places = Vec::new();
    let (mut total, mut rays, mut per_frame, mut far, mut nearest) = (0, 32, 0, 1f32, f32::MAX);
    for (slot, (_, volume)) in gi.each().enumerate() {
        let span = volume.max - volume.min;
        let span = [span.x, span.y, span.z];
        let step: [f32; 3] = std::array::from_fn(|i| {
            if volume.counts[i] > 1 {
                span[i] / (volume.counts[i] - 1) as f32
            } else {
                0.
            }
        });
        for (i, &s) in step.iter().enumerate() {
            if volume.counts[i] > 1 && s > 0. {
                nearest = nearest.min(s);
            }
        }
        let first = volume.probe_position([0; 3]);
        let (lo, hi, fade) = reach(volume, step);
        let diagonal = (step[0] * step[0] + step[1] * step[1] + step[2] * step[2]).sqrt();
        far = far.max(diagonal * 1.5);
        let bias = 0.3
            * step
                .iter()
                .copied()
                .filter(|s| *s > 0.)
                .fold(f32::MAX, f32::min)
                .min(1.);
        set.volumes[slot] = VolumeUniform {
            origin: [first.x, first.y, first.z, total as f32],
            spacing: [step[0], step[1], step[2], 0.],
            inverse: [
                if step[0] > 0. { 1. / step[0] } else { 0. },
                if step[1] > 0. { 1. / step[1] } else { 0. },
                if step[2] > 0. { 1. / step[2] } else { 0. },
                bias,
            ],
            counts: [volume.counts[0], volume.counts[1], volume.counts[2], 0],
            lo: [lo[0], lo[1], lo[2], 0.],
            hi: [hi[0], hi[1], hi[2], 0.],
            fade: [fade[0], fade[1], fade[2], 0.],
        };
        let c = volume.counts;
        for local in 0..volume.probes() {
            let at = [local % c[0], (local / c[0]) % c[1], local / (c[0] * c[1])];
            places.push(std::array::from_fn(|i| {
                [first.x, first.y, first.z][i] + at[i] as f32 * step[i]
            }));
        }
        total += volume.probes();
        rays = rays.max(volume.rays);
        per_frame += volume.per_frame;
        set.count[0] += 1;
    }
    let light = atlas(device, total, LIGHT_TILE, "3D probe light");
    let depth = atlas(device, total, DEPTH_TILE, "3D probe distance");
    set.texel = [
        1. / light.size[0] as f32,
        1. / light.size[1] as f32,
        1. / depth.size[0] as f32,
        1. / depth.size[1] as f32,
    ];
    let set = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("3D probe volumes"),
            contents: bytemuck::bytes_of(&set),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let nearest = if nearest == f32::MAX { 1. } else { nearest };
    Layout {
        of: gi.layout,
        total,
        rays,
        per_frame: per_frame.min(total).max(1),
        far,
        lift: (nearest * 0.01).max(1e-4),
        light,
        depth,
        set,
        places,
        cleared: false,
    }
}

/// Where `volume` (its probes `step` apart) lights fully, corner to corner,
/// and how far past that its light fades out, by axis. A box of probes is
/// laid just inside what it lights (a room's walls and floor), so it
/// lights fully half a spacing past its outer probes and fades over the
/// next half; an axis of one probe lights its box and fades over a
/// twentieth of it.
fn reach(volume: &crate::ProbeVolume, step: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (min, max) = (volume.min.to_array(), volume.max.to_array());
    let lo = std::array::from_fn(|i| min[i] - step[i] * 0.5);
    let hi = std::array::from_fn(|i| max[i] + step[i] * 0.5);
    let fade = std::array::from_fn(|i| {
        if step[i] > 0. {
            step[i] * 0.5
        } else {
            ((max[i] - min[i]) * 0.05).max(1e-3)
        }
    });
    (lo, hi, fade)
}

/// Group 0 of a probe-lit pipeline: the frame's (`frame::environment_layout`
/// where `lit`), then the probes' atlases, volumes and sampler.
fn make_frame_layout(device: &wgpu::Device, lit: bool) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
    ];
    if lit {
        entries.extend(environment_entries(wgpu::ShaderStages::FRAGMENT));
    }
    entries.extend(probe_entries(wgpu::ShaderStages::FRAGMENT));
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("3D frame, probe-lit"),
        entries: &entries,
    })
}
fn environment_entries(stage: wgpu::ShaderStages) -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: stage,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::Cube,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: stage,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}
fn probe_entries(stage: wgpu::ShaderStages) -> [wgpu::BindGroupLayoutEntry; 4] {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    [
        texture(9),
        texture(10),
        wgpu::BindGroupLayoutEntry {
            binding: 11,
            visibility: stage,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 12,
            visibility: stage,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}

/// What a tracer's binding holds: the scene's generation, the
/// environment, the layout and the rays' buffer's size.
type TraceKey = (u64, Option<u64>, u64, u64, u64);
/// What a probe-lit frame's binding holds: whether shadows are on, the
/// environment, the layout.
type FrameKey = (bool, Option<u64>, u64);

/// What the faces' bindings hold: the layout, the lights' and rays'
/// buffers, the environment, whether the sun's shadow is on.
type RasterKey = (u64, u64, u64, Option<u64>, bool, u64, Option<wgpu::Buffer>);

impl Tracer {
    /// Counts the times what the probes see of the meshes changed.
    fn revision(&self) -> u64 {
        match (&self.scene, &self.raster) {
            (Some(scene), _) => scene.revision,
            (_, Some(raster)) => raster.revision,
            _ => 0,
        }
    }
    /// Whether it holds the world's meshes yet.
    fn built(&self) -> bool {
        match (&self.scene, &self.raster) {
            (Some(scene), _) => scene.meshes.is_some(),
            (_, Some(raster)) => raster.meshes.is_some(),
            _ => false,
        }
    }
}

/// What updates probes by rays: the scene, the tracer's and blender's
/// pipelines, and their buffers.
struct Tracer {
    /// Where the device has ray queries: the scene as rays see it, and the
    /// tracer's group 0 (without an environment and with).
    scene: Option<Scene>,
    trace_layouts: Option<[wgpu::BindGroupLayout; 2]>,
    /// Where it hasn't: the probes' faces drawn instead (`probes_raster`),
    /// and their bindings with what they were made for.
    raster: Option<super::probes_raster::Raster>,
    raster_binds: Option<(RasterKey, [wgpu::BindGroup; 2])>,
    trace: [Option<wgpu::ComputePipeline>; 2],
    blend_layout: wgpu::BindGroupLayout,
    blend: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    /// Each ray's light and distance, for as many as a frame casts.
    rays: Option<(wgpu::Buffer, u64)>,
    /// The tracer's binding, and what it was made for (the scene's
    /// generation, the environment, the layout).
    bind: Option<(TraceKey, wgpu::BindGroup)>,
    /// The blender's, light then distance, with the layout and rays they
    /// were made for.
    blend_binds: Option<((u64, u64), [wgpu::BindGroup; 2])>,
    /// On wgpu's no-op backend (`world_bench`), which sizes acceleration
    /// structures at nothing: wgpu then records no build, and a pass
    /// tracing them fails. Everything but that pass runs.
    noop: bool,
    /// Every point and spot light (`trace_lights`), its count, and how many
    /// times it was made again (what binds it is made again with it).
    lights: wgpu::Buffer,
    light_count: u32,
    lights_made: u64,
}

/// `lights` into the tracer's buffer: made again where it outgrew it.
fn upload_lights(
    tracer: &mut Tracer,
    lights: &[TraceLight],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    let bytes: &[u8] = bytemuck::cast_slice(lights);
    if bytes.len() as u64 > tracer.lights.size() {
        tracer.lights = light_buffer(device, bytes.len() as u64);
        tracer.lights_made += 1;
    }
    if !bytes.is_empty() {
        queue.write_buffer(&tracer.lights, 0, bytes);
    }
    tracer.light_count = lights.len() as u32;
}
fn light_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D probe lights"),
        // A light at least: a binding isn't empty.
        size: size
            .max(std::mem::size_of::<TraceLight>() as u64)
            .next_power_of_two(),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(super) struct Probes {
    layout: Option<Layout>,
    sampler: wgpu::Sampler,
    /// Probe-lit group 0 layouts (without an environment and with), and the
    /// frame's binding with what it was made for (whether shadows are on,
    /// the environment, the layout).
    frame_layouts: [Option<wgpu::BindGroupLayout>; 2],
    pub(super) bind: Option<(FrameKey, wgpu::BindGroup)>,
    tracer: Option<Tracer>,
    /// The next probe to update; rounds finished since the last change and
    /// probes updated in this one; the light and meshes they're for. A
    /// round's probes all blend by its weight and turn their rays by its
    /// rotation, so what the probes converge to after a change doesn't
    /// depend on what came before it but through light bounced in its
    /// first rounds.
    next: u32,
    rounds: u32,
    done: u32,
    lit_by: Option<([f32; 20], u64, Option<u64>)>,
    /// The point and spot lights rays are lit by, as uploaded, and this
    /// frame's (`trace_lights`).
    lights: Vec<TraceLight>,
    seen: Vec<TraceLight>,
    /// The world's `Gi::poison` and `Gi::face_budget`, as of this frame.
    poison: u32,
    face_budget: u64,
}

/// Whether `world`'s meshes take their diffuse light from probes on this
/// device this frame: it has volumes and the device traces rays or draws
/// faces (`probes_raster`). Where it has volumes and not, says so
/// (`World::probes_status`).
pub(super) fn probed(world: &Inner, device: &wgpu::Device) -> bool {
    if !world.gi.any() {
        return false;
    }
    if !super::raytrace::ray_queries(device) && !super::probes_raster::rasterises(device) {
        world.gi.status.set(crate::ProbesStatus::Unsupported);
        return false;
    }
    true
}

/// The probes' part of a frame layout shading through clusters or
/// cascades as well (`lights::make_layouts`).
#[cfg(feature = "lighting")]
pub(super) fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
    probe_entries(wgpu::ShaderStages::FRAGMENT)
}
/// And of its binding (`lights::make_bind`): none before the probes are.
#[cfg(feature = "lighting")]
pub(super) fn bind_entries(gpu: &Gpu) -> Option<[wgpu::BindGroupEntry<'_>; 4]> {
    let probes = gpu.probes.as_ref()?;
    let layout = probes.layout.as_ref()?;
    Some([
        wgpu::BindGroupEntry {
            binding: 9,
            resource: wgpu::BindingResource::TextureView(&layout.light.view),
        },
        wgpu::BindGroupEntry {
            binding: 10,
            resource: wgpu::BindingResource::TextureView(&layout.depth.view),
        },
        wgpu::BindGroupEntry {
            binding: 11,
            resource: layout.set.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::Sampler(&probes.sampler),
        },
    ])
}

/// Group 0's layout for a probe-lit pipeline (`lit`: with the environment
/// as well) made, with the first; `frame_layout` reads it.
pub(super) fn frame_layout_of(gpu: &mut Gpu, device: &wgpu::Device, lit: bool) {
    let probes = gpu
        .probes
        .as_mut()
        .expect("probes are made before their pipelines");
    probes.frame_layouts[usize::from(lit)].get_or_insert_with(|| make_frame_layout(device, lit));
}
pub(super) fn frame_layout(gpu: &Gpu, lit: bool) -> &wgpu::BindGroupLayout {
    let probes = gpu
        .probes
        .as_ref()
        .expect("probes are made before their pipelines");
    probes.frame_layouts[usize::from(lit)].as_ref().unwrap()
}

/// Prepares and updates `world`'s probes for this frame, into `encoder`
/// before its passes: the scene made again where `drawn` (this frame's
/// meshes, where it made them) says the meshes changed. Returns whether
/// the frame is probe-lit; drops everything where it's not.
pub(super) fn frame(
    gpu: &mut Gpu,
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    encoder: &mut wgpu::CommandEncoder,
    world: &Inner,
    uniforms: &FrameUniforms,
    (probed, drawn): (bool, Option<&[Drawn]>),
) -> bool {
    if !probed {
        // The hold goes whether or not these probes held it: a renderer
        // made again (samples, format) has none, while the world may
        // still be held by the last one's.
        gpu.probes = None;
        world.unsettle(SETTLE, false);
        return false;
    }
    // The renderer's frame uniforms and environment, beside its probes.
    let Gpu {
        probes: slot,
        uniforms: frame_buffer,
        environment: cube,
        environment_layout,
        shadow_map,
        stand_in,
        sampler,
        #[cfg(feature = "lighting")]
        lights,
        ..
    } = gpu;
    let probes = slot.get_or_insert_with(|| Probes {
        layout: None,
        sampler: device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3D probes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
        frame_layouts: [None, None],
        bind: None,
        tracer: None,
        next: 0,
        rounds: 0,
        done: 0,
        lit_by: None,
        lights: Vec::new(),
        seen: Vec::new(),
        poison: 0,
        face_budget: super::probes_raster::BUDGET,
    });
    probes.poison = world.gi.poison;
    probes.face_budget = world.gi.face_budget.unwrap_or(super::probes_raster::BUDGET);
    if probes
        .layout
        .as_ref()
        .is_none_or(|l| l.of != world.gi.layout)
    {
        probes.layout = Some(layout(device, world));
        probes.bind = None;
        // Clusters' and cascades' bindings hold the atlases too.
        #[cfg(feature = "lighting")]
        lights.remade();
        probes.next = 0;
        probes.rounds = 0;
        probes.done = 0;
        if let Some(tracer) = &mut probes.tracer {
            tracer.bind = None;
            tracer.blend_binds = None;
            tracer.raster_binds = None;
        }
    }
    let layout = probes.layout.as_mut().unwrap();
    if !layout.cleared {
        for atlas in [&layout.light, &layout.depth] {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D probes cleared"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &atlas.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        layout.cleared = true;
    }
    // Rays where the device has them, unless the world asks for faces.
    let rays = super::raytrace::ray_queries(device) && !world.gi.rasterised;
    if probes
        .tracer
        .as_ref()
        .is_some_and(|t| t.scene.is_some() != rays)
    {
        probes.tracer = None;
        probes.lit_by = None;
    }
    let tracer = probes.tracer.get_or_insert_with(|| tracer(device, rays));
    if let Some(drawn) = drawn {
        if let Some(scene) = &mut tracer.scene
            && scene.meshes != Some(world.meshes)
        {
            scene.rebuild((device, queue), encoder, drawn, world.meshes);
        }
        if let Some(raster) = &mut tracer.raster
            && raster.meshes != Some(world.meshes)
        {
            raster.rebuild((device, queue), drawn, world.meshes);
        }
    }
    // A change to what rays see or what lights it: converge again. (Not
    // the camera: nothing here follows it.)
    trace_lights(world, &mut probes.seen);
    let lights_changed = probes.seen != probes.lights;
    if lights_changed {
        std::mem::swap(&mut probes.seen, &mut probes.lights);
        upload_lights(tracer, &probes.lights, device, queue);
    }
    let environment = world.environment.as_ref().map(|e| e.data.id);
    let now = (lighting(uniforms), tracer.revision(), environment);
    if lights_changed || probes.lit_by.as_ref() != Some(&now) {
        probes.lit_by = Some(now);
        probes.rounds = 0;
        probes.done = 0;
    }
    world.gi.status.set(if tracer.scene.is_some() {
        crate::ProbesStatus::Traced
    } else {
        crate::ProbesStatus::Rasterised
    });
    let tracer = probes.tracer.as_ref().unwrap();
    if probes.rounds < ROUNDS && tracer.built() {
        let environment = match (environment, cube.as_ref(), environment_layout.as_ref()) {
            (Some(id), Some((_, view)), Some((_, sampler))) => Some((id, view, sampler)),
            _ => None,
        };
        #[cfg(feature = "lighting")]
        let cascades = lights.sun.as_ref().map(|sun| (&sun.uniforms, &sun.array));
        #[cfg(not(feature = "lighting"))]
        let cascades = None;
        let shadow = Shadow {
            map: shadow_map.as_ref().unwrap_or(stand_in),
            sampler,
            on: shadow_map.is_some(),
            cascades,
        };
        update(
            probes,
            (frame_buffer, shadow),
            environment,
            (device, queue),
            encoder,
        );
    }
    world.unsettle(SETTLE, probes.rounds < ROUNDS);
    true
}

fn tracer(device: &wgpu::Device, rays: bool) -> Tracer {
    let compute = wgpu::ShaderStages::COMPUTE;
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: compute,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let trace_layout = |lit: bool| {
        let mut entries = vec![uniform(0, compute)];
        if lit {
            entries.extend(environment_entries(compute));
        }
        entries.extend(probe_entries(compute));
        entries.extend([
            wgpu::BindGroupLayoutEntry {
                binding: 16,
                visibility: compute,
                ty: wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
                count: None,
            },
            storage(17, true),
            storage(18, true),
            storage(19, true),
            storage(20, false),
            uniform(21, compute),
            storage(22, true),
        ]);
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D probe rays"),
            entries: &entries,
        })
    };
    let fragment = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("3D probe blend"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            uniform(1, fragment),
            uniform(2, fragment),
        ],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("3D probe blend"),
        source: wgpu::ShaderSource::Wgsl(include_str!("probes_blend.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("3D probe blend"),
        bind_group_layouts: &[Some(&blend_layout)],
        immediate_size: 0,
    });
    let weighted = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Constant,
        dst_factor: wgpu::BlendFactor::OneMinusConstant,
        operation: wgpu::BlendOperation::Add,
    };
    let blend = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D probe blend"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("tile"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("blend"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState {
                    color: weighted,
                    alpha: weighted,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    // Drawing faces instead: the capture's group 0 (the frame's shadow,
    // the probes', the updaters' uniform and lights) and the reading's.
    let raster = (!rays).then(|| {
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let mut entries = vec![
            uniform(0, both),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ];
        entries.extend(probe_entries(both));
        entries.extend([
            uniform(21, both),
            wgpu::BindGroupLayoutEntry {
                binding: 22,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]);
        // Under the sun's cascades, their uniforms and maps as well.
        let capture = |cascaded: bool| {
            let mut entries = entries.clone();
            if cascaded {
                entries.extend([
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                ]);
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("3D probe faces"),
                entries: &entries,
            })
        };
        let read = |lit: bool| {
            let mut entries = vec![uniform(0, compute)];
            if lit {
                entries.extend(environment_entries(compute));
            }
            entries.extend([
                storage(20, false),
                uniform(21, compute),
                wgpu::BindGroupLayoutEntry {
                    binding: 23,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ]);
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("3D probe faces read"),
                entries: &entries,
            })
        };
        super::probes_raster::Raster::new((
            [capture(false), capture(true)],
            [read(false), read(true)],
        ))
    });
    Tracer {
        scene: rays.then(|| Scene::new(device)),
        trace_layouts: rays.then(|| [trace_layout(false), trace_layout(true)]),
        raster,
        raster_binds: None,
        trace: [None, None],
        blend_layout,
        blend,
        uniform: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D probe rays"),
            size: std::mem::size_of::<TraceUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        rays: None,
        bind: None,
        blend_binds: None,
        noop: device.adapter_info().backend == wgpu::Backend::Noop,
        lights: light_buffer(device, 0),
        light_count: 0,
        lights_made: 0,
    }
}

/// The tracer's WGSL: the frame's declarations, the probes', the
/// updaters' shared part and its own, tags taken off (`updater_source`).
#[cold]
fn trace_source(lit: bool) -> String {
    updater_source(
        "enable wgpu_ray_query;\n",
        (lit, false),
        &[
            include_str!("shader/frame.wgsl"),
            include_str!("shader/probes.wgsl"),
            include_str!("probes_common.wgsl"),
            include_str!("probes_trace.wgsl"),
        ],
    )
}
/// The rasteriser's WGSL (`probes_raster.rs`): as the tracer's, its own
/// part in place of the tracer's, with the sun's cascades
/// (`shader/shadows.wgsl`) where `cascaded`.
#[cold]
pub(super) fn raster_source(lit: bool, cascaded: bool) -> String {
    updater_source(
        "",
        (lit, cascaded),
        &[
            include_str!("shader/frame.wgsl"),
            include_str!("shader/probes.wgsl"),
            include_str!("shader/shadows.wgsl"),
            include_str!("probes_common.wgsl"),
            include_str!("probes_raster.wgsl"),
        ],
    )
}
/// `parts` after `head`, tags taken off: `//probed ` lines kept, `//E `
/// lines kept where `lit`, `//H ` where `cascaded` and `//I ` where not,
/// other tagged lines dropped.
#[cold]
fn updater_source(head: &str, (lit, cascaded): (bool, bool), parts: &[&str]) -> String {
    let mut out = String::from(head);
    for text in parts {
        for line in text.lines() {
            // A tag is `//` and a capital or a name (lowercase letters,
            // digits, `_`), then a space: the shader's (`shader.rs`).
            let tag = line.strip_prefix("//").and_then(|rest| {
                let end = rest.find(' ')?;
                let tag = &rest[..end];
                let letter = tag.len() == 1 && tag.as_bytes()[0].is_ascii_uppercase();
                let name = tag.len() > 1
                    && tag
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                (letter || name).then(|| (tag, &rest[end + 1..]))
            });
            let line = match tag {
                Some(("probed", rest)) => rest,
                Some(("E", rest)) if lit => rest,
                Some(("H", rest)) if cascaded => rest,
                Some(("I", rest)) if !cascaded => rest,
                Some(_) => continue,
                None => line,
            };
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// A rotation turning round `n`'s rays, the same for the same `n`.
fn turn(n: u32) -> [[f32; 4]; 4] {
    let hash = |x: u32| {
        let mut h = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
        ((h >> 22) ^ h) as f32 / u32::MAX as f32
    };
    // A random unit quaternion (Shoemake).
    let (u1, u2, u3) = (hash(n * 3), hash(n * 3 + 1), hash(n * 3 + 2));
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1. - u1).sqrt(), u1.sqrt());
    let q = crate::Quat {
        x: a * (tau * u2).sin(),
        y: a * (tau * u2).cos(),
        z: b * (tau * u3).sin(),
        w: b * (tau * u3).cos(),
    };
    crate::Mat4::from_transform(crate::Vec3::ZERO, q, crate::vec3(1., 1., 1.)).columns
}

/// One frame's update: this frame's probes' rays traced, then blended
/// into their maps by the round's weight.
fn update(
    probes: &mut Probes,
    (frame, shadow): (&wgpu::Buffer, Shadow),
    environment: Option<(u64, &wgpu::TextureView, &wgpu::Sampler)>,
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    encoder: &mut wgpu::CommandEncoder,
) {
    let layout = probes.layout.as_ref().unwrap();
    let tracer = probes.tracer.as_mut().unwrap();
    // Never past the round's end: the next round blends by another weight.
    // Drawing faces, at most `MOST` a frame.
    let most = if tracer.raster.is_some() {
        super::probes_raster::MOST
    } else {
        u32::MAX
    };
    let mut count = layout.per_frame.min(layout.total - probes.done).min(most);
    // Drawing faces: as many of those as fit the budget, each face culled.
    if let Some(raster) = &mut tracer.raster {
        let places: Vec<[f32; 3]> = (0..count)
            .map(|k| layout.places[((probes.next + k) % layout.total) as usize])
            .collect();
        let cut = super::probes_raster::cut(layout.far);
        count = raster.plan((device, queue), &places, cut, probes.face_budget);
    }
    let size = u64::from(layout.per_frame * layout.rays) * 16;
    if tracer.rays.as_ref().is_none_or(|(_, have)| *have < size) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D probe rays"),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        tracer.rays = Some((buffer, size));
        tracer.bind = None;
        tracer.blend_binds = None;
    }
    let uniform = TraceUniform {
        turn: turn(probes.rounds),
        first: probes.next,
        count,
        rays: layout.rays,
        total: layout.total,
        far: layout.far,
        lift: layout.lift,
        lights: tracer.light_count,
        seed: probes.rounds,
        poison: probes.poison,
        zero: 0.,
        _pad: [0; 2],
    };
    queue.write_buffer(&tracer.uniform, 0, bytemuck::bytes_of(&uniform));
    let atlases = Atlases {
        light: &layout.light.view,
        depth: &layout.depth.view,
        set: &layout.set,
        sampler: &probes.sampler,
    };
    if tracer.raster.is_some() {
        rasterise(
            tracer,
            (device, encoder),
            (frame, shadow),
            environment,
            atlases,
            (layout.of, count, layout.rays),
        );
    } else {
        trace(
            tracer,
            (device, encoder),
            frame,
            environment,
            atlases,
            (layout.of, count, layout.rays),
        );
    }
    let (rays, rays_size) = tracer.rays.as_ref().unwrap();
    let key = (layout.of, *rays_size);
    if tracer.blend_binds.as_ref().is_none_or(|(k, _)| *k != key) {
        let bind = |atlas: &Atlas| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D probe blend"),
                layout: &tracer.blend_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: rays.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: tracer.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: atlas.uniform.as_entire_binding(),
                    },
                ],
            })
        };
        tracer.blend_binds = Some((key, [bind(&layout.light), bind(&layout.depth)]));
    }
    let weight = f64::from((1. / (probes.rounds + 1) as f32).max(FLOOR));
    let binds = &tracer.blend_binds.as_ref().unwrap().1;
    for (atlas, bind) in [&layout.light, &layout.depth].into_iter().zip(binds) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D probe blend"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &atlas.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&tracer.blend);
        pass.set_bind_group(0, bind, &[]);
        pass.set_blend_constant(wgpu::Color {
            r: weight,
            g: weight,
            b: weight,
            a: weight,
        });
        pass.draw(0..4, 0..count);
    }
    probes.next = (probes.next + count) % layout.total;
    probes.done += count;
    if probes.done == layout.total {
        probes.done = 0;
        probes.rounds += 1;
    }
}

/// What a probe-lit pass reads of the probes: their atlases, volumes and
/// sampler.
#[derive(Clone, Copy)]
struct Atlases<'a> {
    light: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
    set: &'a wgpu::Buffer,
    sampler: &'a wgpu::Sampler,
}
/// The frame's shadow map (or stand-in), its sampler, whether the sun
/// casts through it, and the sun's cascades' uniforms and maps where it
/// casts through them instead.
#[derive(Clone, Copy)]
pub(super) struct Shadow<'a> {
    map: &'a wgpu::TextureView,
    sampler: &'a wgpu::Sampler,
    on: bool,
    cascades: Option<(&'a wgpu::Buffer, &'a wgpu::TextureView)>,
}

/// This frame's probes' faces drawn and read along their rays into the
/// rays buffer (`probes_raster`).
fn rasterise(
    tracer: &mut Tracer,
    (device, encoder): (&wgpu::Device, &mut wgpu::CommandEncoder),
    (frame, shadow): (&wgpu::Buffer, Shadow),
    environment: Option<(u64, &wgpu::TextureView, &wgpu::Sampler)>,
    atlases: Atlases,
    (of, count, rays_per): (u64, u32, u32),
) {
    let lit = environment.is_some();
    let (rays, rays_size) = tracer.rays.as_ref().unwrap();
    let raster = tracer.raster.as_mut().unwrap();
    raster.hold(device, count);
    let cascaded = shadow.cascades.is_some();
    let key = (
        of,
        tracer.lights_made,
        *rays_size,
        environment.map(|e| e.0),
        shadow.on,
        raster.made,
        shadow.cascades.map(|(uniforms, _)| uniforms.clone()),
    );
    if tracer.raster_binds.as_ref().is_none_or(|(k, _)| *k != key) {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shadow.map),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(shadow.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(atlases.light),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(atlases.depth),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: atlases.set.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(atlases.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 21,
                resource: tracer.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 22,
                resource: tracer.lights.as_entire_binding(),
            },
        ];
        if let Some((uniforms, maps)) = shadow.cascades {
            entries.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: uniforms.as_entire_binding(),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(maps),
            });
        }
        let capture = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D probe faces"),
            layout: raster.capture_layout(cascaded),
            entries: &entries,
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: frame.as_entire_binding(),
        }];
        if let Some((_, view, sampler)) = environment {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 20,
                resource: rays.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 21,
                resource: tracer.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 23,
                resource: wgpu::BindingResource::TextureView(raster.faces()),
            },
        ]);
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D probe faces read"),
            layout: raster.read_layout(lit),
            entries: &entries,
        });
        tracer.raster_binds = Some((key, [capture, read]));
    }
    let [capture, read] = &tracer.raster_binds.as_ref().unwrap().1;
    raster.update(
        device,
        encoder,
        (capture, read),
        (lit, cascaded, count, rays_per),
    );
}

/// This frame's probes' rays traced into the rays buffer.
fn trace(
    tracer: &mut Tracer,
    (device, encoder): (&wgpu::Device, &mut wgpu::CommandEncoder),
    frame: &wgpu::Buffer,
    environment: Option<(u64, &wgpu::TextureView, &wgpu::Sampler)>,
    atlases: Atlases,
    (of, count, rays_per): (u64, u32, u32),
) {
    let lit = environment.is_some();
    let (rays, rays_size) = tracer.rays.as_ref().unwrap();
    let trace_layouts = tracer
        .trace_layouts
        .as_ref()
        .expect("traced where the device has ray queries");
    let pipeline = tracer.trace[usize::from(lit)].get_or_insert_with(|| {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D probe rays"),
            source: wgpu::ShaderSource::Wgsl(trace_source(lit).into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D probe rays"),
            bind_group_layouts: &[Some(&trace_layouts[usize::from(lit)])],
            immediate_size: 0,
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("3D probe rays"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("trace_rays"),
            compilation_options: Default::default(),
            cache: None,
        })
    });
    let scene = tracer.scene.as_ref().unwrap();
    let key = (
        scene.generation,
        environment.map(|e| e.0),
        of,
        *rays_size,
        tracer.lights_made,
    );
    if tracer.bind.as_ref().is_none_or(|(k, _)| *k != key) {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: frame.as_entire_binding(),
        }];
        if let Some((_, view, sampler)) = environment {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(atlases.light),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(atlases.depth),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: atlases.set.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(atlases.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 16,
                resource: scene.tlas.as_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 17,
                resource: scene.vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 18,
                resource: scene.indices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 19,
                resource: scene.instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 20,
                resource: rays.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 21,
                resource: tracer.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 22,
                resource: tracer.lights.as_entire_binding(),
            },
        ]);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D probe rays"),
            layout: &trace_layouts[usize::from(lit)],
            entries: &entries,
        });
        tracer.bind = Some((key, bind));
    }
    if !tracer.noop {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("3D probe rays"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &tracer.bind.as_ref().unwrap().1, &[]);
        pass.dispatch_workgroups((count * rays_per).div_ceil(64), 1, 1);
    }
}

/// The frame's binding for probe-lit pipelines (`lit`: with the
/// environment's cube), made again as what it holds changes.
pub(super) fn frame_bind<'a>(
    gpu: &'a mut Gpu,
    device: &wgpu::Device,
    lit: bool,
) -> &'a wgpu::BindGroup {
    let Gpu {
        probes,
        uniforms,
        shadow_map,
        stand_in,
        sampler,
        environment,
        environment_layout,
        ..
    } = gpu;
    let probes = probes.as_mut().expect("a probe-lit frame has probes");
    let cube = match (lit, environment.as_ref(), environment_layout.as_ref()) {
        (true, Some((id, view)), Some((_, cube_sampler))) => Some((*id, view, cube_sampler)),
        _ => None,
    };
    let layout_of = probes.layout.as_ref().map_or(0, |l| l.of);
    let key = (shadow_map.is_some(), cube.map(|c| c.0), layout_of);
    if probes.bind.as_ref().is_none_or(|(k, _)| *k != key) {
        let layout = probes.layout.as_ref().unwrap();
        let group = probes.frame_layouts[usize::from(cube.is_some())]
            .get_or_insert_with(|| make_frame_layout(device, cube.is_some()));
        let map = shadow_map.as_ref().unwrap_or(stand_in);
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(map),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ];
        if let Some((_, view, cube_sampler)) = cube {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(cube_sampler),
            });
        }
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&layout.light.view),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(&layout.depth.view),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: layout.set.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::Sampler(&probes.sampler),
            },
        ]);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D frame, probe-lit"),
            layout: group,
            entries: &entries,
        });
        probes.bind = Some((key, bind));
    }
    &probes.bind.as_ref().unwrap().1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Variant, shader::wgsl};

    /// A device of wgpu's no-op backend that traces rays (it validates
    /// shaders and runs nothing).
    fn device() -> wgpu::Device {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = super::super::testing::now(instance.request_adapter(&Default::default()))
            .expect("the no-op adapter");
        let descriptor = wgpu::DeviceDescriptor {
            required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            required_limits: wgpu::Limits::default()
                .using_minimum_supported_acceleration_structure_values(),
            // SAFETY: the no-op backend runs nothing.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        };
        super::super::testing::now(adapter.request_device(&descriptor))
            .expect("the no-op device")
            .0
    }

    #[test]
    fn every_probe_shader_is_valid_wgsl() {
        let device = device();
        // (wgpu's handler panics on an invalid shader.)
        let module = |label: &str, source: String| {
            let _ = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        };
        for lit in [false, true] {
            module("rays", trace_source(lit));
            module("faces", raster_source(lit, false));
            module("faces under cascades", raster_source(lit, true));
            for other in [0, Variant::TEXTURED, Variant::TEXTURED | Variant::COATED] {
                let variant = Variant(Variant::PROBES | other).with(Variant::LIT, lit);
                module("mesh", wgsl(variant, 4));
            }
        }
        module("blend", include_str!("probes_blend.wgsl").into());
    }

    #[test]
    fn probes_hold_no_volume_by_value() {
        // DXC 1.9 at -Od (wgpu's debug builds on Direct3D 12) crashes
        // compiling a struct copied whole out of a uniform array inside a
        // loop that returns: the probes' volumes are read through a
        // pointer instead. Here, no shader of theirs loads a `ProbeVolume`
        // whole (naga's IR: a load whose value is that struct).
        use zgx_gpu::wgpu::naga;
        let whole_loads = |source: &str| {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let mut found = 0;
            let mut look = |function: &naga::Function, fun_info: &naga::valid::FunctionInfo| {
                for (handle, expression) in function.expressions.iter() {
                    if let naga::Expression::Load { .. } = expression
                        && let naga::TypeInner::Struct { .. } =
                            fun_info[handle].ty.inner_with(&module.types)
                        && let naga::proc::TypeResolution::Handle(ty) = &fun_info[handle].ty
                        && module.types[*ty].name.as_deref() == Some("ProbeVolume")
                    {
                        found += 1;
                    }
                }
            };
            for (handle, function) in module.functions.iter() {
                look(function, &info[handle]);
            }
            for (i, entry) in module.entry_points.iter().enumerate() {
                look(&entry.function, info.get_entry_point(i));
            }
            found
        };
        for lit in [false, true] {
            assert_eq!(whole_loads(&trace_source(lit)), 0, "the tracer");
            assert_eq!(whole_loads(&raster_source(lit, false)), 0, "the faces");
            assert_eq!(
                whole_loads(&raster_source(lit, true)),
                0,
                "the faces under cascades"
            );
            let variant = Variant(Variant::PROBES).with(Variant::LIT, lit);
            assert_eq!(whole_loads(&wgsl(variant, 4)), 0, "a probe-lit mesh");
        }
        // (It finds one where there is one.)
        let copy = trace_source(false).replace(
            "let volume = &probe_set.volumes[i];",
            "let volume = probe_set.volumes[i];",
        );
        assert!(whole_loads(&copy) > 0);
    }

    #[test]
    fn a_move_on_the_face_path_writes_its_instances_alone() {
        // Drawing faces (a device without ray queries: wgpu's no-op one
        // here), a mesh moving rewrites the meshes' instances and makes no
        // buffer: each geometry's vertices stay as they were uploaded.
        use crate::{Geometry, Material, Object, ProbeVolume, Transform, World};
        use zgx_gpu::viewport::{Viewport, ViewportFrame};
        let (device, queue) = super::super::testing::device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::sphere(1., 64), Material::default()));
        world.add(Object::mesh(Geometry::plane(10., 10.), Material::default()));
        let cube = world.add(
            Object::mesh(Geometry::cuboid(0.3, 0.3, 0.3), Material::default()).at([2., 0.5, 0.]),
        );
        world.set_probes_rasterised(true);
        world
            .add_probes(ProbeVolume::new([-3., 0., -3.], [3., 2., 3.], [4, 3, 4]))
            .unwrap();
        let mut renderer = world.renderer();
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let draw = |renderer: &mut super::super::Renderer| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut frame =
                ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (64, 64), 1.);
            renderer.render(&mut frame);
            queue.submit([encoder.finish()]);
        };
        let counts = |renderer: &super::super::Renderer| {
            let probes = renderer.gpu.as_ref().unwrap().probes.as_ref().unwrap();
            let raster = probes
                .tracer
                .as_ref()
                .unwrap()
                .raster
                .as_ref()
                .expect("faces");
            (
                raster.buffers_made,
                raster.instance_bytes,
                raster.cluster_bytes,
                raster.plans,
            )
        };
        draw(&mut renderer);
        let (made, written, clustered, _) = counts(&renderer);
        // The geometries' indices, reordered, and cluster tables: the
        // sphere's 4,096 triangles at least.
        assert!(clustered > 4_096 * 12, "{clustered} bytes");
        // Three geometries' vertices and indices, the instances', and the
        // list of meshes each face's draws take.
        assert_eq!(made, 8);
        world.set_transform(cube, Transform::at([2.2, 0.5, 0.]));
        draw(&mut renderer);
        let (made_after, written_after, clustered_after, _) = counts(&renderer);
        assert_eq!(made_after, made, "a move made a buffer");
        assert_eq!(clustered_after, clustered, "a move clustered again");
        assert_eq!(
            written_after - written,
            3 * std::mem::size_of::<super::super::probes_raster::Instance>() as u64,
            "a move wrote more than the three meshes' instances"
        );
        // Converged (16 rounds, a round a frame), a world at rest culls
        // and draws no faces.
        for _ in 0..40 {
            draw(&mut renderer);
        }
        let (.., plans) = counts(&renderer);
        assert!(plans >= 16, "{plans} frames culled");
        for _ in 0..5 {
            draw(&mut renderer);
        }
        assert_eq!(counts(&renderer).3, plans, "a world at rest culled faces");
    }

    #[test]
    fn a_volume_lights_half_a_spacing_past_its_outer_probes() {
        // Probes 0.4 apart from -1 to 1 across, one probe up: the walls at
        // ±1.2 are lit fully, light fades out by ±1.4; up, the box alone.
        let volume = crate::ProbeVolume::new([-1., 0., -1.], [1., 2., 1.], [6, 1, 6]);
        let (lo, hi, fade) = reach(&volume, [0.4, 0., 0.4]);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(near(lo, [-1.2, 0., -1.2]), "{lo:?}");
        assert!(near(hi, [1.2, 2., 1.2]), "{hi:?}");
        assert!(near(fade, [0.2, 0.1, 0.2]), "{fade:?}");
    }

    #[test]
    fn a_volume_lays_out_its_probes_in_the_atlases() {
        let world = crate::World::new();
        world
            .add_probes(crate::ProbeVolume::new(
                [-1., 0., -1.],
                [1., 2., 1.],
                [5, 3, 1],
            ))
            .unwrap();
        world
            .add_probes(crate::ProbeVolume::new(
                [4., 0., 0.],
                [6., 1., 2.],
                [8, 8, 2],
            ))
            .unwrap();
        let device = device();
        let inner = world.inner();
        let layout = layout(&device, &inner);
        assert_eq!(layout.total, 15 + 128);
        // 143 probes: three rows of 64.
        assert_eq!(layout.light.size, [512, 24]);
        assert_eq!(layout.depth.size, [1024, 48]);
        assert_eq!(layout.per_frame, 128);
    }
}
