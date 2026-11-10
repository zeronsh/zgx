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
#[cfg(feature = "lighting")]
mod clusters;
mod cull;
#[cfg(feature = "shaders")]
mod custom;
mod draw;
mod frame;
mod gpu;
mod graph;
mod instances;
#[cfg(feature = "lighting")]
mod lights;
mod materials;
mod pipelines;
#[cfg(feature = "gi")]
mod probes;
#[cfg(feature = "gi")]
mod raytrace;
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
#[cfg(feature = "shaders")]
pub(crate) use custom::engine_text;
/// Custom materials: none without the `shaders` feature.
#[cfg(not(feature = "shaders"))]
mod custom {
    /// Whether a run's custom material reads what is behind: none does.
    pub(super) fn reads(_: &super::Gpu) -> bool {
        false
    }
    /// Whether a custom material moves its vertices: none does.
    pub(super) fn boxes_needed(_: &crate::world::Inner) -> bool {
        false
    }
}
use draw::{Run, draw_main};
use frame::frame_uniforms;
use gpu::{Buffers, Gpu, Kept};
use graph::Pass;
use instances::{Instance, instance, morph_block, note, write_changed};
use materials::{make_binding, map_set, placed, upload_missing};
use pipelines::make_main_pipelines;
#[cfg(feature = "lighting")]
use shadows::sun_bounds;
use shadows::{
    Casting, SHADOW_SIZE, draw_shadows, make_shadow_pipeline, shadow_key, sun_view, sun_view_within,
};
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
    /// A frame's poses, kept for the next to fill again (the meshes' list
    /// borrows them, apart from `scratch`).
    poses: crate::world::Poses,
    scratch: Scratch,
}
impl Renderer {
    pub(crate) fn new(world: Weak<RefCell<Inner>>) -> Self {
        Self {
            world,
            gpu: None,
            poses: Default::default(),
            scratch: Scratch::default(),
        }
    }
}
/// A frame's lists, kept for the next to fill again: a frame allocates
/// none of its own, so what it costs doesn't follow where the heap is.
#[derive(Default)]
struct Scratch {
    all: Vec<crate::world::Drawn<'static>>,
    sorted: Vec<&'static crate::world::Drawn<'static>>,
    order: Vec<((u64, [u64; MAPS]), usize)>,
    palette: Vec<[[f32; 4]; 4]>,
    /// What the instance buffer holds, kept from frame to frame, and which
    /// of them this frame changed.
    instances: Vec<Instance>,
    changed: Vec<std::ops::Range<usize>>,
    /// Each instance's mesh's bounding sphere, and the frame's spans of each
    /// run the camera may see (`cull`).
    boxes: cull::Boxes,
    /// The world's count of changes (`Inner::meshes`) `boxes` are as of, if
    /// they're whole.
    boxed: Option<u64>,
    mask: Vec<bool>,
    /// Frames to draw without culling, the last that culled having left
    /// out almost nothing.
    unculled: u32,
    shown: Vec<(u32, std::ops::Range<u32>)>,
    /// The world's count of changes (`Inner::meshes`) when `instances` were
    /// last made, and of those reordering (`Inner::ordering`) when `order`
    /// was, if no key read the camera.
    prepared: u64,
    ordered: Option<u64>,
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

/// A frame's meshes when what they draw changed (`Inner::meshes`): sorted
/// (the list returned), their instances and poses uploaded, their
/// buffers and bindings made and marked used, in runs (`Scratch::runs`).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn prepare<'a>(
    gpu: &mut Gpu,
    scratch: &mut Scratch,
    all: &'a [crate::world::Drawn<'a>],
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    world: &Inner,
    view: Mat4,
    (lit, transmits, lighting): (bool, bool, u16),
) -> Vec<&'a crate::world::Drawn<'a>> {
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
    // Nothing changed that its keys read, and no key reads the camera: the
    // last frame's order holds, keys and all.
    let unmoved = scratch.ordered == Some(world.ordering) && order.len() == all.len();
    let kept_order = unmoved
        || order.len() == all.len() && {
            for (k, i) in order.iter_mut() {
                *k = key(&all[*i]);
            }
            order.is_sorted()
        };
    if !kept_order {
        if order.len() != all.len() {
            order.clear();
            order.extend(all.iter().map(key).zip(0..));
        }
        order.sort_unstable();
    }
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
    // The instances the buffer holds, kept from frame to frame. In the same
    // order as the last frame's, a mesh not stamped since (`Drawn::stamp`)
    // holds its instance already, one stamped is made again and written;
    // posed ones always are (their poses move). In another order each is
    // made again and written where it differs. The ranges written are noted
    // for the upload.
    let instances = &mut scratch.instances;
    let since = scratch.prepared;
    let changed = &mut scratch.changed;
    changed.clear();
    let mut pose = |d: &crate::world::Drawn| {
        let base = palette.len() as u32;
        if morphing(d) {
            morph_block(d, palette);
        }
        if skinned(d) {
            palette.extend(d.palette.iter().copied().flatten().map(|m| m.columns));
        }
        base
    };
    let stale = |d: &crate::world::Drawn| d.stamp > since || skinned(d) || morphing(d);
    if kept_order
        && instances.len() == drawn.len()
        && drawn.iter().filter(|d| stale(d)).count() * 2 <= drawn.len()
    {
        // Few changed: those made again and written.
        for (at, d) in drawn.iter().enumerate() {
            let posed = skinned(d) || morphing(d);
            if !posed && d.stamp <= since {
                continue;
            }
            let base = if posed { pose(d) } else { 0 };
            instances[at] = instance(d, base);
            note(changed, at);
        }
    } else if kept_order && instances.len() == drawn.len() {
        // Most changed (a world turning): all made again, written whole.
        instances.clear();
        instances.extend(drawn.iter().map(|d| {
            let posed = skinned(d) || morphing(d);
            instance(d, if posed { pose(d) } else { 0 })
        }));
        changed.push(0..drawn.len());
    } else {
        changed_order(instances, changed, &drawn, |d| {
            let posed = skinned(d) || morphing(d);
            instance(d, if posed { pose(d) } else { 0 })
        });
    }
    scratch.prepared = world.meshes;
    if !palette.is_empty() {
        let bytes: &[u8] = bytemuck::cast_slice(palette);
        if gpu
            .palettes
            .as_ref()
            .is_none_or(|(_, capacity, _)| *capacity < bytes.len())
        {
            let capacity = bytes.len().next_power_of_two().max(4096);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D joints"),
                size: capacity as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D joints"),
                layout: &gpu.palette_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            gpu.palettes = Some((buffer, capacity, bind));
        }
        queue.write_buffer(&gpu.palettes.as_ref().unwrap().0, 0, bytes);
        gpu.written += bytes.len() as u64;
    }
    if !instances.is_empty() {
        let bytes: &[u8] = bytemuck::cast_slice(instances);
        // The buffer holds the last frame's: only what changed goes up,
        // unless it's made anew.
        let kept = gpu
            .instances
            .as_ref()
            .is_some_and(|(_, capacity)| *capacity >= bytes.len());
        if !kept {
            let capacity = bytes.len().next_power_of_two().max(4096);
            gpu.instances = Some((
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D instances"),
                    size: capacity as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                capacity,
            ));
        }
        let buffer = &gpu.instances.as_ref().unwrap().0;
        gpu.written += if kept {
            write_changed(queue, buffer, instances, changed)
        } else {
            queue.write_buffer(buffer, 0, bytes);
            bytes.len() as u64
        };
    }
    // What they draw with, kept and marked used. (Meshes sharing it are
    // mostly neighbours by now.) Maps not yet uploaded are noted, and
    // uploaded together after.
    let mut missing = Vec::new();
    let mut last = None;
    for (d, ((_, set), _)) in drawn.iter().zip(order.iter()) {
        if last == Some((d.geometry.id(), set)) {
            continue;
        }
        last = Some((d.geometry.id(), set));
        let entry = gpu.geometries.entry(d.geometry.id()).or_insert_with(|| {
            use wgpu::util::DeviceExt;
            Buffers {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("3D vertices"),
                    contents: bytemuck::cast_slice(d.geometry.vertices()),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("3D indices"),
                    contents: bytemuck::cast_slice(d.geometry.indices()),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                skin: d.geometry.skin().map(|skin| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("3D joints and weights"),
                        contents: bytemuck::cast_slice(skin),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                }),
                second_uvs: None,
                colors: d.geometry.colors().map(|colors| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("3D vertex colours"),
                        contents: bytemuck::cast_slice(colors),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                }),
                morphs: d.geometry.morphs().filter(|_| skinning).map(|morphs| {
                    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("3D morph targets"),
                        contents: bytemuck::cast_slice(morphs.deltas()),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            entry.second_uvs = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("3D second texture coordinates"),
                    contents: bytemuck::cast_slice(uvs),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            );
        }
        for (map, texture) in crate::Map::ALL.into_iter().zip(d.maps) {
            if let Some(texture) = texture {
                // A texture that can't be read draws as if it had none.
                let key = (texture.id(), map.srgb());
                match gpu.views.get_mut(&key) {
                    Some(entry) => entry.1 = gpu.frame,
                    None if !texture.failed() && !gpu.loads.coming(key) => {
                        missing.push((texture, map.srgb()));
                    }
                    None => {}
                }
            }
        }
    }
    if !missing.is_empty() {
        upload_missing(gpu, (device, queue), missing);
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
        let binding = make_binding(gpu, device, queue, d);
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
    // A clustered or cascaded world's pipelines read its lights' lists or
    // cascades (`lighting`, `Variant` bits): its runs' alone, so the
    // meshes' loop above reads nothing more.
    if lighting != 0 {
        for run in runs.iter_mut() {
            run.variant = Variant(run.variant.0 | lighting);
        }
    }
    scratch.ordered = (!runs.iter().any(|run| run.glass)).then_some(world.ordering);
    drawn
}

/// `instances` made again for `drawn` in another order than the last
/// frame's (`made` makes one): each kept where it's what the buffer holds
/// at its place, the rest noted in `changed`. Apart from `prepare`: rare.
#[cold]
#[inline(never)]
fn changed_order(
    instances: &mut Vec<Instance>,
    changed: &mut Vec<std::ops::Range<usize>>,
    drawn: &[&crate::world::Drawn],
    mut made: impl FnMut(&crate::world::Drawn) -> Instance,
) {
    instances.truncate(drawn.len());
    let held = instances.len();
    for (at, d) in drawn.iter().enumerate() {
        let made = made(d);
        if at >= held {
            instances.push(made);
        } else if bytemuck::bytes_of(&instances[at]) == bytemuck::bytes_of(&made) {
            continue;
        } else {
            instances[at] = made;
        }
        note(changed, at);
    }
}

/// What the shadow map would hold now, the sun's view `shadow` onto the
/// opaque meshes of `drawn` (the frame's sorted list): drawn only if it
/// holds something else. Their pipelines made.
// Once a frame, out of its loop: inlined, the frame's code compiled worse.
#[inline(never)]
fn casting_of(
    gpu: &mut Gpu,
    scratch: &mut Scratch,
    drawn: &[&crate::world::Drawn],
    shadow: Option<(Mat4, f32)>,
    device: &wgpu::Device,
) -> Option<Casting> {
    let translucent = |d: &crate::world::Drawn| d.material.color[3] < 1.;
    let (palette, runs) = (&scratch.palette, &scratch.runs);
    // What the map would hold now; drawn only if it holds something else.
    let spare = &mut scratch.casting;
    let casting = shadow.map(|(matrix, _)| {
        let (_, mut meshes, mut joints, mut weights, mut cuts) = spare.take().unwrap_or_default();
        meshes.clear();
        joints.clear();
        weights.clear();
        cuts.clear();
        let opaque = || drawn.iter().filter(|d| !translucent(d));
        meshes.extend(opaque().map(|d| {
            let cutoff = d.surface.alpha_cutoff.map_or(0, f32::to_bits);
            if cutoff != 0 {
                cuts.push(cut_key(d));
            }
            (d.geometry.id(), d.matrix, cutoff)
        }));
        // Poses: an empty palette has none, so no mesh need be asked.
        if !palette.is_empty() {
            joints.extend(opaque().flat_map(|d| d.palette.iter().copied().flatten().copied()));
            weights.extend(opaque().flat_map(|d| d.morph.iter().copied().flatten().copied()));
        }
        (matrix, meshes, joints, weights, cuts)
    });
    if casting.is_some() {
        for run in runs.iter() {
            let key = shadow_key(run.variant);
            if run.glass || key == 0 || gpu.shadow_variants.contains_key(&key) {
                continue;
            }
            make_shadow_pipeline(gpu, device, run.variant);
        }
    }
    casting
}

/// What a cut-out shadow reads of `d` beside its place: its maps (the base
/// colour's alpha cuts), their placement and what binds with them, as its
/// run keys them (`map_set`); 0 where it casts whole (no base colour map,
/// or light passes through it, as `shadow_key` says). Asked of casters with
/// a cutoff alone. (Reading the set the frame's sort made instead cost every
/// caster more than this costs masked ones: the loop compiled worse.)
#[cold]
#[inline(never)]
fn cut_key(d: &crate::world::Drawn) -> u64 {
    let glass = d.extras.is_some_and(|e| e.kinds & crate::world::GLASS != 0);
    if d.maps[0].is_some() && !glass {
        map_set(d)[0]
    } else {
        0
    }
}

impl Viewport for Renderer {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let Some(world) = self.world.upgrade() else {
            return;
        };
        #[cfg(feature = "gltf")]
        crate::world::animation::frame(&world, frame.time);
        let world = world.borrow();
        world.frames.set(world.frames.get() + 1);
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
        // Maps loaded since the last frame (`crate::assets`): bound in place
        // of the stand-ins their meshes drew with, and shadows drawn again
        // (a cut-out cast its shadow whole without its base colour).
        if gpu.loads.receive(&mut gpu.views) {
            gpu.bindings.clear();
            gpu.behind_held = 0;
            gpu.kept = None;
            gpu.shadow_drawn = None;
            #[cfg(feature = "lighting")]
            if let Some(sun) = &mut gpu.lights.sun {
                sun.held = Default::default();
            }
        }
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
            // Bindings of what was behind before, and runs drawing with them.
            gpu.bindings.clear();
            gpu.kept = None;
            gpu.behind_held = 0;
        }

        let camera = world.camera;
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let view = camera.view();
        let view_projection = camera.projection(aspect) * view;
        let mut uniforms = frame_uniforms(&world, view_projection, encode);
        #[cfg(feature = "lighting")]
        let (clustered, cascaded) = lights::prepare(
            gpu,
            &world,
            (view, camera.projection(aspect)),
            size,
            (frame.device, frame.queue),
            &mut uniforms,
        );
        #[cfg(not(feature = "lighting"))]
        let (clustered, cascaded) = (false, false);
        // Diffuse light from probes (`probes.rs`). Not yet beside clusters
        // or cascades: those take group 0's other bindings.
        #[cfg(feature = "gi")]
        let probed = !clustered && !cascaded && probes::probed(&world, frame.device);
        #[cfg(not(feature = "gi"))]
        let probed = false;
        let lighting = if clustered { Variant::CLUSTERED } else { 0 }
            | if cascaded { Variant::CASCADED } else { 0 }
            | if probed { Variant::PROBES } else { 0 };
        let scratch = &mut self.scratch;
        // The sun's single map's (none while its shadow is in cascades).
        let sun = (world.shadows && !cascaded)
            .then(|| {
                let [x, y, z, _] = uniforms.sun_direction;
                (uniforms.sun_color != [0.; 4]).then_some(crate::vec3(x, y, z))
            })
            .flatten();
        // Where only how it's seen changed, the meshes draw as the last frame
        // drew them: its runs, instances and shadow map. Not with
        // translucent ones, whose order follows the camera.
        let kept = gpu
            .kept
            .filter(|k| k.meshes == world.meshes && k.sun == sun && !k.translucent);
        let lit = world.environment.is_some();
        let mut all = recycle(std::mem::take(&mut scratch.all));
        gpu.written = 0;
        // Whether this frame culls: where a frame's culling left out almost
        // nothing (gaps drawn in their spans), the next frames don't look
        // (`cull::RESTING`): drawing all of them is the same frame.
        let culls = world.culling && scratch.unculled == 0;
        scratch.unculled = scratch.unculled.saturating_sub(1);
        let (shadow, casting) = match kept {
            Some(kept) => (kept.shadow, None),
            None => {
                world.drawn_into(&mut self.poses, &mut all);
                // What culling's boxes follow (`cull::update`): as the last
                // frame made instances, and whether the order is unmoved.
                let since = scratch.prepared;
                let unmoved =
                    scratch.ordered == Some(world.ordering) && scratch.order.len() == all.len();
                let drawn = prepare(
                    gpu,
                    scratch,
                    &all,
                    (frame.device, frame.queue),
                    &world,
                    view,
                    (lit, transmits, lighting),
                );
                // Cascades cull their casters by the boxes, resting or not.
                // (And where a custom material moves its vertices: its boxes,
                // grown, are what the sun's view fits.)
                if culls || cascaded || custom::boxes_needed(&world) {
                    let skinning = gpu.skinning && world.posing();
                    cull::update(scratch, &drawn, (since, unmoved), world.meshes, skinning);
                    #[cfg(feature = "shaders")]
                    custom::grow_boxes(gpu, scratch, &drawn);
                } else {
                    scratch.boxed = None;
                }
                // The sun's view, fit to the opaque meshes' boxes where this
                // frame made them.
                let opaque = scratch.runs.iter().filter(|run| !run.glass);
                let ranges =
                    opaque.map(|run| run.instances.start as usize..run.instances.end as usize);
                let shadow = sun.and_then(|sun| {
                    if scratch.boxed == Some(world.meshes) {
                        sun_view_within(&all, scratch.boxes.union(ranges), sun)
                    } else {
                        sun_view(&all, sun)
                    }
                });
                let casting = casting_of(gpu, scratch, &drawn, shadow, frame.device);
                // The cascades' casters' box, where it may have changed.
                #[cfg(feature = "lighting")]
                if cascaded {
                    let opaque = scratch.runs.iter().filter(|run| !run.glass);
                    let ranges =
                        opaque.map(|run| run.instances.start as usize..run.instances.end as usize);
                    lights::cascade_scene(
                        gpu,
                        match scratch.boxed == Some(world.meshes) {
                            true => scratch.boxes.union(ranges),
                            false => sun_bounds(&all),
                        },
                    );
                }
                scratch.sorted = recycle(drawn);
                (shadow, casting)
            }
        };
        if let Some((matrix, texel)) = shadow {
            uniforms.shadow = matrix.columns;
            // The lookup's offset off a surface: 1.5 texels facing the sun,
            // up to 6 more turned from it (`sunlit`).
            uniforms.shadow_texel = [texel, 1. / SHADOW_SIZE as f32, texel * 7.5, texel * 6.];
            uniforms.params[3] = 1.;
        }
        gpu.shadows(frame.device, world.shadows && !cascaded);
        if let Some(environment) = world.environment.as_ref() {
            gpu.environment(frame.device, frame.queue, environment);
        }
        #[cfg(feature = "lighting")]
        lights::bind(gpu, frame.device, lit);
        #[cfg(feature = "gi")]
        probes::frame(
            gpu,
            (frame.device, frame.queue),
            frame.encoder,
            &world,
            &uniforms,
            (probed, kept.is_none().then_some(&all[..])),
        );
        frame
            .queue
            .write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&uniforms));
        gpu.written += std::mem::size_of::<frame::FrameUniforms>() as u64;
        if kept.is_none() {
            gpu.kept = Some(Kept {
                meshes: world.meshes,
                sun,
                shadow,
                translucent: scratch.runs.iter().any(|run| run.glass),
            });
        }
        let (runs, spare) = (&scratch.runs, &mut scratch.casting);
        // What of each run the camera may see. Where a frame's culling left
        // out almost nothing (gaps drawn in their spans), the next frames
        // don't look (`cull::RESTING`): drawing all of them is the same frame.
        // (With boxes as the meshes are: a frame keeping the last's culls
        // only if the last made them.)
        let culling = culls && scratch.boxed == Some(world.meshes);
        let mask = culling.then(|| {
            let frustum = cull::Frustum::new(&view_projection);
            cull::visible(&frustum, &scratch.boxes, &mut scratch.mask);
            &scratch.mask[..]
        });
        let geometries = &gpu.geometries;
        let indices = |run: &Run| geometries[&run.geometry].count;
        let culled = cull::shown(runs, mask, &mut scratch.shown, indices);
        if culling && cull::hardly(culled, scratch.instances.len()) {
            scratch.unculled = cull::RESTING;
        }
        world.culled.set(culled);
        // The sun's cascades: fit to the view, their casters culled, which
        // to draw again decided.
        #[cfg(feature = "lighting")]
        if cascaded {
            let boxes = (scratch.boxed == Some(world.meshes)).then_some(&scratch.boxes);
            let changed: &[std::ops::Range<usize>] = match kept {
                Some(_) => &[],
                None => &scratch.changed,
            };
            lights::cascades(
                gpu,
                &world,
                (aspect, &uniforms),
                (boxes, changed, scratch.instances.len()),
                runs,
                (frame.device, frame.queue),
            );
        }
        let shown = &scratch.shown;
        let mut casting = casting;
        for run in runs.iter() {
            let variant = run.variant;
            if variant == Variant::default() || gpu.variants.contains_key(&variant) {
                continue;
            }
            make_main_pipelines(gpu, frame.device, variant);
        }
        // Custom materials' pipelines, numbers and time (none in a world
        // without them).
        #[cfg(feature = "shaders")]
        custom::prepare(gpu, &world, frame, (runs, shown));
        #[cfg(feature = "gi")]
        if probed {
            probes::frame_bind(gpu, frame.device, lit);
        }
        let frame_bind = match &gpu.environment_bind {
            #[cfg(feature = "gi")]
            _ if probed => &gpu.probes.as_ref().unwrap().bind.as_ref().unwrap().1,
            Some(bind) if lit => bind,
            _ => &gpu.bind,
        };
        #[cfg(feature = "lighting")]
        let frame_bind = gpu.lights.frame_bind(lit).unwrap_or(frame_bind);
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
                    #[cfg(feature = "lighting")]
                    if cascaded {
                        lights::draw_cascades(frame.encoder, gpu, runs);
                    }
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
                    if runs.iter().any(|run| run.variant.transmitting()) || custom::reads(gpu) {
                        draw_behind(frame.encoder, gpu, (runs, shown), frame_bind, clear);
                    }
                }
                Pass::Main => {
                    draw_main(frame.encoder, gpu, (runs, shown), target, frame_bind, clear)
                }
            }
        }
        if casting.is_some() {
            *spare = casting;
        }
        scratch.all = recycle(all);
        world.temporaries.set(gpu.temporary_bytes());
        // Frames keep coming while maps are on their way.
        world.unsettle(crate::world::Settle::LOADING, gpu.loads.pending());
    }
    fn device_changed(&mut self) {
        self.gpu = None;
    }
    fn rest(&mut self) {
        // The frame's temporaries go, and the bindings holding what was
        // behind glass with them: the next frame makes them again.
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        gpu.graph.rest();
        gpu.bindings.clear();
        gpu.behind_held = 0;
        // The last frame's runs drew with those bindings: the next frame
        // makes its meshes' anew.
        gpu.kept = None;
        if let Some(world) = self.world.upgrade() {
            world.borrow().temporaries.set(gpu.temporary_bytes());
        }
    }
}

/// What the renderer's tests draw with: wgpu's no-op backend.
#[cfg(test)]
mod testing {
    use zgx_gpu::wgpu;

    /// `future`, of wgpu's no-op backend, which never waits.
    pub(super) fn now<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the no-op backend waited"),
        }
    }

    /// A device of wgpu's no-op backend and its queue: textures made, none
    /// drawn.
    pub(super) fn device_and_queue() -> (wgpu::Device, wgpu::Queue) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = now(instance.request_adapter(&Default::default()));
        let adapter = adapter.expect("the no-op adapter");
        now(adapter.request_device(&Default::default())).expect("the no-op device")
    }
    /// Its device alone.
    pub(super) fn device() -> wgpu::Device {
        device_and_queue().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Geometry, Light, Material, Object, Transform, World};
    use zgx_gpu::viewport::Viewport;

    /// A frame drawn at 64 by 64 by `renderer`: the bytes it wrote into
    /// buffers.
    fn written(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) -> u64 {
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
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(device, queue, &mut encoder, &view, format, (64, 64), 1.);
        renderer.render(&mut frame);
        queue.submit([encoder.finish()]);
        renderer.gpu.as_ref().unwrap().written
    }

    #[test]
    fn a_frame_where_only_the_camera_moved_writes_the_frames_uniforms_alone() {
        let (device, queue) = testing::device_and_queue();
        let world = World::new();
        world.set_shadows(true);
        world.add_light(Light::Directional {
            direction: crate::vec3(-0.5, -1., -0.4),
            color: [1.; 3],
            intensity: 2.,
        });
        let cubes: Vec<_> = (0..16)
            .map(|i| {
                let cube = world.add(Object::mesh(
                    Geometry::cuboid(1., 1., 1.),
                    Material::rgb(0x808080),
                ));
                world.set_transform(cube, Transform::at([i as f32 * 2., 0., 0.]));
                cube
            })
            .collect();
        let mut renderer = world.renderer();
        let uniforms = std::mem::size_of::<frame::FrameUniforms>() as u64;
        let instance = std::mem::size_of::<Instance>() as u64;
        assert_eq!(
            written(&mut renderer, &device, &queue),
            uniforms + 16 * instance,
            "the first frame: every instance"
        );
        world.set_camera(Camera::looking_at([3., 4., 9.], [8., 0., 0.]));
        assert_eq!(
            written(&mut renderer, &device, &queue),
            uniforms,
            "the camera alone"
        );
        world.set_transform(cubes[5], Transform::at([10., 1., 0.]));
        assert_eq!(
            written(&mut renderer, &device, &queue),
            uniforms + instance,
            "one mesh moved: its instance"
        );
        world.set_camera(Camera::looking_at([3., 4., 8.], [8., 0., 0.]));
        assert_eq!(written(&mut renderer, &device, &queue), uniforms);
    }

    #[test]
    fn culling_rests_while_it_leaves_out_almost_nothing() {
        let (device, queue) = testing::device_and_queue();
        let world = World::new();
        world.set_camera(Camera::looking_at([0., 0., 30.], [0., 0., 0.]));
        // 99 in view, one far behind the camera: 1%.
        for i in 0..100 {
            let cube = world.add(Object::mesh(
                Geometry::cuboid(1., 1., 1.),
                Material::rgb(0x808080),
            ));
            let at = if i == 0 {
                [0., 0., 500.]
            } else {
                [(i % 10) as f32 - 5., (i / 10) as f32 - 5., 0.]
            };
            world.set_transform(cube, Transform::at(at));
        }
        let mut renderer = world.renderer();
        let culled: Vec<u32> = (0..18)
            .map(|_| {
                written(&mut renderer, &device, &queue);
                world.culled()
            })
            .collect();
        let mut rested = vec![1];
        rested.extend([0; cull::RESTING as usize]);
        rested.extend([1, 0]);
        assert_eq!(culled, rested);
    }
}
