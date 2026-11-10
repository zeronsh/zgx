//! Drawing a `World`: its view's painter. A frame is one pass with depth and
//! 4x multisampling (or 1 sample, `World::set_samples`): each geometry's
//! meshes in one instanced draw, opaque ones first, then the translucent
//! back to front. Pipelines and buffers are made with the first frame and
//! kept; a geometry's buffers are uploaded once and dropped a while after
//! the last frame drawing it. With shadows, a pass before it draws the
//! opaque meshes' depth as the sun sees them.
//!
//! Its parts: `gpu` (what a renderer keeps on the device), `frame` (the
//! frame's uniforms and binding), `instances` (a mesh's per-instance data),
//! `variant` (what sets a pipeline apart), `pipelines`, `shader` (the WGSL
//! and its variants), `materials` (group 1: maps and layers), `shadows`
//! (the sun's map), `behind` (what light passing through reads) and `draw`
//! (runs of meshes in a pass).
mod behind;
mod draw;
mod frame;
mod gpu;
mod graph;
mod instances;
mod materials;
mod pipelines;
mod shader;
mod shadows;
mod targets;
mod variant;

use crate::{
    math::{Mat4, Vec3},
    texture::MAPS,
    world::Inner,
};
use behind::{Behind, draw_behind};
use draw::{Run, draw_main};
use frame::frame_uniforms;
use gpu::{Buffers, Gpu};
use graph::Pass;
use instances::{Instance, instance, morph_block};
use materials::{make_binding, map_set, placed, upload};
use pipelines::make_main_pipelines;
use shadows::{Casting, SHADOW_SIZE, draw_shadows, make_shadow_pipeline, shadow_key, sun_view};
use std::{cell::RefCell, rc::Weak};
use variant::Variant;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Frames a geometry's buffers stay after the last that drew it.
const KEEP_FRAMES: u64 = 240;

pub(crate) struct Renderer {
    world: Weak<RefCell<Inner>>,
    gpu: Option<Gpu>,
    scratch: Scratch,
}
impl Renderer {
    pub(crate) fn new(world: Weak<RefCell<Inner>>) -> Self {
        Self {
            world,
            gpu: None,
            scratch: Scratch::default(),
        }
    }
}
/// A frame's lists, kept for the next to fill again: a frame allocates
/// none of its own, so what it costs doesn't follow where the heap is.
#[derive(Default)]
struct Scratch {
    poses: crate::world::Poses,
    all: Vec<crate::world::Drawn<'static>>,
    sorted: Vec<&'static crate::world::Drawn<'static>>,
    order: Vec<((u64, [u64; MAPS]), usize)>,
    palette: Vec<[[f32; 4]; 4]>,
    instances: Vec<Instance>,
    runs: Vec<Run>,
    /// What the shadow map held before, its lists to fill again.
    casting: Option<Casting>,
}
/// `list` emptied, as a list of another type of one size: its allocation
/// kept (a `Vec` collected in place from its own iterator reuses it).
fn recycle<T, U>(mut list: Vec<T>) -> Vec<U> {
    list.clear();
    list.into_iter().map(|_| unreachable!()).collect()
}

impl Viewport for Renderer {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let Some(world) = self.world.upgrade() else {
            return;
        };
        let world = world.borrow();
        // Shade in linear light: written through the sRGB view where there is
        // one, encoded by the shader where not.
        let (target, format, encode) = match frame.target_srgb {
            Some(srgb) => (srgb, frame.format.add_srgb_suffix(), false),
            None => (frame.target, frame.format, !frame.format.is_srgb()),
        };
        let samples = world.samples;
        if self
            .gpu
            .as_ref()
            .is_none_or(|gpu| gpu.format != format || gpu.samples != samples)
        {
            self.gpu = Some(Gpu::new(frame.device, format, samples));
        }
        let gpu = self.gpu.as_mut().unwrap();
        gpu.frame += 1;
        let size = frame.size;
        // The frame's passes, compiled again only as what they follow
        // changes.
        let transmits = world.transmits();
        let config = graph::Config {
            size,
            shadows: world.shadows,
            transmits,
        };
        if gpu.graph.prepare(frame.device, config) {
            // Bindings of what was behind before.
            gpu.bindings.clear();
        }

        let camera = world.camera;
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let view = camera.view();
        let view_projection = camera.projection(aspect) * view;
        let mut uniforms = frame_uniforms(&world, view_projection, encode);
        let scratch = &mut self.scratch;
        let mut all = recycle(std::mem::take(&mut scratch.all));
        world.drawn_into(&mut scratch.poses, &mut all);
        let sun = world
            .shadows
            .then(|| {
                let [x, y, z, _] = uniforms.sun_direction;
                (uniforms.sun_color != [0.; 4]).then_some(crate::vec3(x, y, z))
            })
            .flatten();
        let shadow = sun.and_then(|sun| sun_view(&all, sun));
        if let Some((matrix, texel)) = shadow {
            uniforms.shadow = matrix.columns;
            // The lookup's offset off a surface: 1.5 texels facing the sun,
            // up to 6 more turned from it (`sunlit`).
            uniforms.shadow_texel = [texel, 1. / SHADOW_SIZE as f32, texel * 7.5, texel * 6.];
            uniforms.params[3] = 1.;
        }
        gpu.shadows(frame.device, world.shadows);
        let lit = world.environment.as_ref();
        if let Some(environment) = lit {
            gpu.environment(frame.device, frame.queue, environment);
        }
        let lit = lit.is_some();
        frame
            .queue
            .write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&uniforms));

        // Meshes by geometry, translucent ones after, back to front: by a
        // key of one integer (a sort of integers, not of meshes).
        let depth = |m: &Mat4| {
            let p = view.project(Vec3::from([
                m.columns[3][0],
                m.columns[3][1],
                m.columns[3][2],
            ]));
            -p.z
        };
        let translucent = |d: &crate::world::Drawn| d.material.color[3] < 1.;
        // Opaque ones by geometry, then maps.
        let key = |d: &crate::world::Drawn| {
            if translucent(d) {
                // Its depth's bits in total order, the farthest least.
                let bits = depth(&d.matrix).to_bits();
                let ordered = if bits >> 31 == 1 {
                    !bits
                } else {
                    bits | 1 << 31
                };
                (1 << 63 | u64::from(!ordered), map_set(d))
            } else {
                (d.geometry.id() & !(1 << 63), map_set(d))
            }
        };
        // Last frame's order with this frame's keys, where it had as many
        // meshes: still in order (most frames), sorting only checks it. Each
        // mesh's index is in its key, so the order is the one a list made
        // anew sorts to.
        let order = &mut scratch.order;
        if order.len() == all.len() {
            for (k, i) in order.iter_mut() {
                *k = key(&all[*i]);
            }
        } else {
            order.clear();
            order.extend(all.iter().map(key).zip(0..));
        }
        order.sort_unstable();
        let mut drawn: Vec<&crate::world::Drawn> = recycle(std::mem::take(&mut scratch.sorted));
        drawn.extend(order.iter().map(|&(_, i)| &all[i]));
        // Skinned meshes' joint matrices, one after another; each mesh's
        // instance says where its own start.
        let skinning = gpu.skinning;
        let posing = skinning && world.posing();
        let skinned = |d: &crate::world::Drawn| posing && d.palette.is_some();
        let morphing = |d: &crate::world::Drawn| posing && d.morph.is_some();
        let palette = &mut scratch.palette;
        palette.clear();
        let instances = &mut scratch.instances;
        instances.clear();
        instances.extend(drawn.iter().map(|d| {
            let base = palette.len() as u32;
            if morphing(d) {
                morph_block(d, palette);
            }
            if skinned(d) {
                palette.extend(d.palette.iter().copied().flatten().map(|m| m.columns));
            }
            instance(d, base)
        }));
        if !palette.is_empty() {
            let bytes: &[u8] = bytemuck::cast_slice(palette);
            if gpu
                .palettes
                .as_ref()
                .is_none_or(|(_, capacity, _)| *capacity < bytes.len())
            {
                let capacity = bytes.len().next_power_of_two().max(4096);
                let buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D joints"),
                    size: capacity as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D joints"),
                    layout: &gpu.palette_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                gpu.palettes = Some((buffer, capacity, bind));
            }
            frame
                .queue
                .write_buffer(&gpu.palettes.as_ref().unwrap().0, 0, bytes);
        }
        if !instances.is_empty() {
            let bytes: &[u8] = bytemuck::cast_slice(instances);
            if gpu
                .instances
                .as_ref()
                .is_none_or(|(_, capacity)| *capacity < bytes.len())
            {
                let capacity = bytes.len().next_power_of_two().max(4096);
                gpu.instances = Some((
                    frame.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("3D instances"),
                        size: capacity as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    capacity,
                ));
            }
            frame
                .queue
                .write_buffer(&gpu.instances.as_ref().unwrap().0, 0, bytes);
        }
        // What they draw with, kept and marked used. (Meshes sharing it are
        // mostly neighbours by now.)
        let mut last = None;
        for (d, ((_, set), _)) in drawn.iter().zip(order.iter()) {
            if last == Some((d.geometry.id(), set)) {
                continue;
            }
            last = Some((d.geometry.id(), set));
            let entry = gpu.geometries.entry(d.geometry.id()).or_insert_with(|| {
                use wgpu::util::DeviceExt;
                Buffers {
                    vertices: frame
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("3D vertices"),
                            contents: bytemuck::cast_slice(d.geometry.vertices()),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                    indices: frame
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("3D indices"),
                            contents: bytemuck::cast_slice(d.geometry.indices()),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                    skin: d.geometry.skin().map(|skin| {
                        frame
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("3D joints and weights"),
                                contents: bytemuck::cast_slice(skin),
                                usage: wgpu::BufferUsages::VERTEX,
                            })
                    }),
                    second_uvs: None,
                    colors: d.geometry.colors().map(|colors| {
                        frame
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("3D vertex colours"),
                                contents: bytemuck::cast_slice(colors),
                                usage: wgpu::BufferUsages::VERTEX,
                            })
                    }),
                    morphs: d.geometry.morphs().filter(|_| skinning).map(|morphs| {
                        let buffer =
                            frame
                                .device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("3D morph targets"),
                                    contents: bytemuck::cast_slice(morphs.deltas()),
                                    usage: wgpu::BufferUsages::STORAGE,
                                });
                        frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("3D morph targets"),
                            layout: &gpu.morph_layout,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buffer.as_entire_binding(),
                            }],
                        })
                    }),
                    count: d.geometry.indices().len() as u32,
                    used: 0,
                }
            });
            entry.used = gpu.frame;
            if placed(d) && entry.second_uvs.is_none() {
                use wgpu::util::DeviceExt;
                let first: Vec<[f32; 2]>;
                let uvs = match d.geometry.second_uvs() {
                    Some(uvs) => uvs,
                    None => {
                        first = d.geometry.vertices().iter().map(|v| v.uv).collect();
                        &first
                    }
                };
                entry.second_uvs = Some(frame.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("3D second texture coordinates"),
                        contents: bytemuck::cast_slice(uvs),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                ));
            }
            for (map, texture) in crate::Map::ALL.into_iter().zip(d.maps) {
                if let Some(texture) = texture {
                    // A texture that can't be read draws as if it had none.
                    let key = (texture.id(), map.srgb());
                    if !gpu.views.contains_key(&key)
                        && let Some(view) = upload(frame.device, frame.queue, texture, map.srgb())
                    {
                        gpu.views.insert(key, (view, 0));
                    }
                    if let Some(entry) = gpu.views.get_mut(&key) {
                        entry.1 = gpu.frame;
                    }
                }
            }
        }
        // Each set of maps bound once, against stand-ins where a map is
        // missing. (Meshes sharing one are mostly neighbours by now.)
        let mut last = &[0; MAPS];
        for (d, ((_, key), _)) in drawn.iter().zip(order.iter()) {
            if std::mem::replace(&mut last, key) == key || *key == [0; MAPS] {
                continue;
            }
            let key = *key;
            if let Some(entry) = gpu.bindings.get_mut(&key) {
                entry.1 = gpu.frame;
                continue;
            }
            let binding = make_binding(gpu, frame.device, frame.queue, d);
            gpu.bindings.insert(key, (binding, gpu.frame));
        }
        let now = gpu.frame;
        gpu.geometries
            .retain(|_, buffers| now - buffers.used <= KEEP_FRAMES);
        gpu.views.retain(|_, (_, used)| now - *used <= KEEP_FRAMES);
        gpu.bindings
            .retain(|_, (_, used)| now - *used <= KEEP_FRAMES);

        // Runs of one geometry and texture, of one pass (opaque, then
        // translucent).
        let variant_of = |d: &crate::world::Drawn| {
            // What it has of light passing through, a coat and a specular,
            // in one byte.
            let kinds = d.extras.map_or(0, |e| e.kinds);
            let bit = |on: bool, bit: u16| if on { bit } else { 0 };
            Variant(
                // Light passing through it, a coat or a specular reads group
                // 1: maps or stand-ins.
                bit(kinds != 0 || d.maps.iter().any(Option::is_some), Variant::TEXTURED)
                    | bit(skinned(d), Variant::SKINNED)
                    | bit(lit, Variant::LIT)
                    | bit(morphing(d), Variant::MORPHING)
                    | bit(d.surface.double_sided, Variant::TWO_SIDED)
                    // Cut by a base colour texture's alpha: without one, whole.
                    | bit(
                        d.surface.alpha_cutoff.is_some() && d.maps[0].is_some(),
                        Variant::MASKED,
                    )
                    | bit(placed(d), Variant::PLACED)
                    | bit(d.geometry.colors().is_some(), Variant::COLORED)
                    // `kinds`' bits, in their order.
                    | (u16::from(kinds) * Variant::TRANSMITTING),
            )
        };
        // Of one geometry and maps, a world without poses or surfaces has
        // one variant.
        let one_variant = !posing && !world.has_surfaces() && !transmits;
        let runs = &mut scratch.runs;
        runs.clear();
        let mut start = 0;
        while start < drawn.len() {
            let first = drawn[start];
            let (geometry, glass, maps, variant) = (
                first.geometry.id(),
                translucent(first),
                order[start].0.1,
                variant_of(first),
            );
            let end = start
                + drawn[start..]
                    .iter()
                    .zip(&order[start..])
                    .take_while(|(d, ((_, set), _))| {
                        d.geometry.id() == geometry
                            && translucent(d) == glass
                            && *set == maps
                            && (one_variant || variant_of(d) == variant)
                    })
                    .count();
            runs.push(Run {
                geometry,
                glass,
                maps,
                variant,
                instances: start as u32..end as u32,
            });
            start = end;
        }
        // What the map would hold now; drawn only if it holds something else.
        let spare = &mut scratch.casting;
        let mut casting = shadow.map(|(matrix, _)| {
            let (_, mut meshes, mut joints, mut weights) = spare.take().unwrap_or_default();
            meshes.clear();
            joints.clear();
            weights.clear();
            let opaque = || drawn.iter().filter(|d| !translucent(d));
            meshes.extend(opaque().map(|d| {
                let cutoff = d.surface.alpha_cutoff.map_or(0, f32::to_bits);
                (d.geometry.id(), d.matrix, cutoff)
            }));
            // Poses: an empty palette has none, so no mesh need be asked.
            if !palette.is_empty() {
                joints.extend(opaque().flat_map(|d| d.palette.iter().copied().flatten().copied()));
                weights.extend(opaque().flat_map(|d| d.morph.iter().copied().flatten().copied()));
            }
            (matrix, meshes, joints, weights)
        });
        if casting.is_some() {
            for run in runs.iter() {
                let key = shadow_key(run.variant);
                if run.glass || key == 0 || gpu.shadow_variants.contains_key(&key) {
                    continue;
                }
                make_shadow_pipeline(gpu, frame.device, run.variant);
            }
        }
        for run in runs.iter() {
            let variant = run.variant;
            if variant == Variant::default() || gpu.variants.contains_key(&variant) {
                continue;
            }
            make_main_pipelines(gpu, frame.device, variant);
        }
        let frame_bind = match &gpu.environment_bind {
            Some(bind) if lit => bind,
            _ => &gpu.bind,
        };
        let clear = world
            .background
            .map_or(wgpu::Color::TRANSPARENT, |[r, g, b, a]| wgpu::Color {
                r: f64::from(r),
                g: f64::from(g),
                b: f64::from(b),
                a: f64::from(a),
            });
        for pass in gpu.graph.passes().iter() {
            match pass {
                Pass::Shadows => {
                    if casting.is_some()
                        && casting != gpu.shadow_drawn
                        && let (Some(map), Some((instances, _))) = (&gpu.shadow_map, &gpu.instances)
                    {
                        draw_shadows(frame.encoder, gpu, runs, map, instances);
                        *spare = std::mem::replace(&mut gpu.shadow_drawn, casting.take());
                    }
                }
                // What is behind meshes light passes through: the others
                // drawn first into a picture of their own, made smaller and
                // smaller for rough surfaces to read blurred.
                Pass::Behind => {
                    if runs.iter().any(|run| run.variant.transmitting()) {
                        draw_behind(frame.encoder, gpu, runs, frame_bind, clear);
                    }
                }
                Pass::Main => draw_main(frame.encoder, gpu, runs, target, frame_bind, clear),
            }
        }
        if casting.is_some() {
            *spare = casting;
        }
        scratch.sorted = recycle(drawn);
        scratch.all = recycle(all);
    }
    fn device_changed(&mut self) {
        self.gpu = None;
    }
    fn rest(&mut self) {
        // The frame's temporaries go; the next frame takes them again.
        if let Some(gpu) = &mut self.gpu {
            gpu.graph.rest();
        }
    }
}
