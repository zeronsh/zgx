//! Point and spot lights past the uniform path's eight: binned into clusters
//! (`clusters`) and shaded each by the fragments its clusters hold
//! (design-3d-aaa.md, 6.3). A world shades this way with more point lights
//! than `POINT_LIGHTS`, any with a radius (`PointLight::radius`), or when
//! asked (`World::set_clustered_lights`); any
//! other draws as it did, its pipelines and frame binding untouched.
//!
//! The lists go up in one storage buffer, read as `vec4<u32>`s (the
//! shader's `clusters`): a header (the grid's scales and slicing, the
//! camera's facing, where each part starts, its tiles and slices), the
//! lights three words each,
//! each cluster's first index and count two to a word, and the indices four
//! to a word. Binned, and written, only on a frame where a light, the
//! camera or the view's size changed.
use super::{
    Gpu,
    clusters::{Binned, CLUSTERS, Grid, Packed, TILES_X, TILES_Y},
};
use crate::{
    math::Mat4,
    world::{Inner, Light, POINT_LIGHTS},
};
use zgx_gpu::wgpu;

/// What the lights' lists on the device were binned for, compared bit for
/// bit (a view not a number is the same view as before, not another).
#[derive(Clone, Copy)]
struct Key {
    view: Mat4,
    projection: Mat4,
    size: (u32, u32),
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        let bits = |m: &Mat4| m.columns.map(|c| c.map(f32::to_bits));
        bits(&self.view) == bits(&other.view)
            && bits(&self.projection) == bits(&other.projection)
            && self.size == other.size
    }
}

/// What a renderer keeps for clustered lights.
#[derive(Default)]
pub(super) struct Lights {
    /// Whether the last frame shaded through clusters.
    on: bool,
    /// The frame's lights, gathered; compared with what was binned.
    gathered: Vec<Packed>,
    binned: Binned,
    key: Option<Key>,
    words: Vec<[u32; 4]>,
    buffer: Option<(wgpu::Buffer, u64)>,
    /// The sun's cascades, while its shadow is cut into them, and those the
    /// device refused.
    pub(super) sun: Option<super::shadows::cascades::Sun>,
    refused: Option<crate::Cascades>,
    /// The frame's layouts with the lists or the cascades, by `key`.
    layouts: [Option<wgpu::BindGroupLayout>; 16],
    /// Their bindings, the renderer's frame binding each was made beside
    /// and the count of this one's own remakes (`made`) then: a shadow
    /// map, an environment, a list's buffer or cascades made again make
    /// them again.
    binds: [Option<(wgpu::BindGroup, Option<wgpu::BindGroup>, u64)>; 16],
    made: u64,
    /// Times the lights were binned, for tests.
    pub(super) binnings: u64,
}

/// The world's point and spot lights as the shader reads them, into
/// `out`: as the uniform path packs its eight (`frame_uniforms`) but for
/// the second word, its colour times its intensity and its radius.
fn gather(world: &Inner, out: &mut Vec<Packed>) {
    out.clear();
    let radii = world.lighting.any_radii();
    world.for_each_light_at(|index, generation, light, spot| {
        if let Light::Point {
            position,
            color,
            intensity,
            range,
        } = light
            && shines(&light)
        {
            let cone = spot.map_or([0., 0., 0., 1.], |spot| {
                let (inner, outer) = (spot.inner.cos(), spot.outer.cos());
                let scale = 1. / (inner - outer).max(1e-3);
                let d = spot.direction.normalized() * scale;
                [d.x, d.y, d.z, -outer * scale]
            });
            let radius = if radii {
                world.lighting.radius(index, generation)
            } else {
                0.
            };
            let [r, g, b] = color.map(|c| c * intensity);
            out.push([
                [position.x, position.y, position.z, range.max(1e-3)],
                [r, g, b, radius],
                cone,
            ]);
        }
    });
}

/// Whether `light` gives any light: one off (no lumens, or black) is left
/// out of what's gathered and binned, and of the uniform path's eight.
pub(super) fn shines(light: &Light) -> bool {
    match light {
        Light::Point {
            color, intensity, ..
        } => *intensity > 0. && color.iter().any(|c| *c > 0.),
        _ => true,
    }
}

/// Whether `world` shades through clusters, on a device that can.
fn wanted(gpu: &Gpu, world: &Inner) -> bool {
    if !gpu.skinning {
        // No storage buffers in its shaders (some GL): the nearest eight.
        return false;
    }
    if world.lighting.clustered || world.has_radii() {
        return true;
    }
    let mut points = 0;
    world.for_each_light(|light, _| {
        points += usize::from(matches!(light, Light::Point { .. }) && shines(&light))
    });
    points > POINT_LIGHTS
}

/// The frame's lights, before its meshes are prepared: whether it shades
/// through clusters (and if so its lists binned, where a light, the camera
/// or the size changed, and uploaded, and the uniform path's lights turned
/// off), and whether the sun's shadow is cut into cascades (their maps
/// made with them, today's single map's lookup turned on). Turning either
/// on or off draws the meshes again (their pipelines differ). Once a
/// frame, out of its loop: inlined, the frame's code compiled worse.
#[inline(never)]
pub(super) fn prepare(
    gpu: &mut Gpu,
    world: &Inner,
    (view, projection): (Mat4, Mat4),
    size: (u32, u32),
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    uniforms: &mut super::frame::FrameUniforms,
) -> (bool, bool) {
    let cascades = world
        .lighting
        .cascades
        .filter(|_| world.shadows && uniforms.sun_color != [0.; 4]);
    // Cascades the device refused (`Sun::new`) aren't asked for again.
    let cascades = cascades.filter(|c| gpu.lights.refused != Some(*c));
    if cascades != gpu.lights.sun.as_ref().map(|sun| sun.settings) {
        gpu.lights.sun = cascades
            .and_then(|c| super::shadows::cascades::Sun::new(device, &gpu.shadow_layout, c));
        gpu.lights.refused = cascades.filter(|_| gpu.lights.sun.is_none());
        gpu.lights.made += 1;
        gpu.kept = None;
    }
    let cascades = cascades.filter(|_| gpu.lights.sun.is_some());
    if cascades.is_some() {
        uniforms.params[3] = 1.;
    } else {
        // No cascades (off, or shadows off): none is owed a redraw, whatever
        // was when they went.
        world.unsettle(crate::world::Settle::CASCADES, false);
    }
    let clustered = clusters(
        gpu,
        world,
        (view, projection),
        size,
        (device, queue),
        uniforms,
    );
    (clustered, cascades.is_some())
}

/// `prepare`'s clusters: whether the frame shades through them.
fn clusters(
    gpu: &mut Gpu,
    world: &Inner,
    (view, projection): (Mat4, Mat4),
    size: (u32, u32),
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    uniforms: &mut super::frame::FrameUniforms,
) -> bool {
    let on = wanted(gpu, world);
    if on != gpu.lights.on {
        gpu.lights.on = on;
        gpu.kept = None;
        if !on {
            gpu.lights.key = None;
        }
    }
    if !on {
        return false;
    }
    uniforms.params[0] = 0.;
    let lights = &mut gpu.lights;
    gather(world, &mut lights.gathered);
    let key = Key {
        view,
        projection,
        size,
    };
    if lights.key == Some(key) && lights.gathered == lights.binned.lights {
        return true;
    }
    lights.key = Some(key);
    lights.binnings += 1;
    let camera = &world.camera;
    let grid = Grid {
        scale: [projection.columns[0][0], projection.columns[1][1]],
        orthographic: camera.orthographic,
        near: camera.near.max(1e-4),
        far: camera.far.max(camera.near.max(1e-4) * 2.),
    };
    lights.binned.bin(&view, grid, &lights.gathered);
    world.lighting.left_out.set(lights.binned.dropped);
    // The words, as the shader reads them.
    let binned = &lights.binned;
    let grid = binned.grid.unwrap_or(grid);
    let [slice_scale, slice_bias] = grid.slicing();
    let facing = (camera.target - camera.position).normalized();
    let f = f32::to_bits;
    let words = &mut lights.words;
    words.clear();
    let base_lights = 4;
    let base_clusters = base_lights + 3 * binned.shown.len() as u32;
    let base_indices = base_clusters + CLUSTERS.div_ceil(2) as u32;
    words.push([
        f(TILES_X as f32 / size.0.max(1) as f32),
        f(TILES_Y as f32 / size.1.max(1) as f32),
        f(slice_scale),
        f(slice_bias),
    ]);
    words.push([
        f(facing.x),
        f(facing.y),
        f(facing.z),
        f(if grid.orthographic { 1. } else { 0. }),
    ]);
    words.push([
        base_lights,
        base_clusters,
        base_indices,
        binned.shown.len() as u32,
    ]);
    words.push([TILES_X, TILES_Y, super::clusters::SLICES, 0]);
    words.extend(binned.shown.iter().flat_map(|l| l.map(|v| v.map(f))));
    words.extend(
        binned
            .clusters
            .chunks(2)
            .map(|p| [p[0][0], p[0][1], p[1][0], p[1][1]]),
    );
    words.extend(binned.indices.chunks(4).map(|c| {
        let mut word = [0; 4];
        word[..c.len()].copy_from_slice(c);
        word
    }));
    let bytes: &[u8] = bytemuck::cast_slice(words);
    if lights
        .buffer
        .as_ref()
        .is_none_or(|(_, capacity)| *capacity < bytes.len() as u64)
    {
        let capacity = (bytes.len() as u64).next_power_of_two().max(1 << 16);
        lights.buffer = Some((make_buffer(device, capacity), capacity));
        lights.made += 1;
    }
    queue.write_buffer(&lights.buffer.as_ref().unwrap().0, 0, bytes);
    gpu.written += bytes.len() as u64;
    true
}

/// `runs` cut where their meshes (`drawn`, in instance order) differ in
/// casting or receiving the sun's shadow (`World::set_casts_shadows`,
/// `set_receives_shadows`), each piece marked: not casting (`Run::casts`)
/// or not receiving (`Variant::UNSHADOWED`). Nothing where no mesh is set
/// either way, the world's table of meshes' extras looked at once.
#[inline(never)]
pub(super) fn shadow_runs(
    runs: &mut Vec<super::Run>,
    drawn: &[&crate::world::Drawn],
    world: &Inner,
) {
    if world.shadow_flags() == 0 {
        return;
    }
    cut_shadow_runs(runs, drawn);
}
#[cold]
#[inline(never)]
fn cut_shadow_runs(runs: &mut Vec<super::Run>, drawn: &[&crate::world::Drawn]) {
    use crate::world::lighting::{NO_CAST, NO_RECEIVE};
    let bits = |at: u32| drawn[at as usize].extras.map_or(0, |e| e.shadows);
    let mut out = Vec::with_capacity(runs.len() + 8);
    for run in runs.drain(..) {
        let mut start = run.instances.start;
        while start < run.instances.end {
            let here = bits(start);
            let end = (start..run.instances.end)
                .find(|&at| bits(at) != here)
                .unwrap_or(run.instances.end);
            out.push(super::Run {
                variant: run
                    .variant
                    .with(super::Variant::UNSHADOWED, here & NO_RECEIVE != 0),
                instances: start..end,
                casts: here & NO_CAST == 0,
                ..run
            });
            start = end;
        }
    }
    *runs = out;
}

/// The single map's casters (`Casting`'s meshes, of the opaque meshes of
/// `drawn` in order) without those set not to cast. Rare: only where one is.
#[cold]
#[inline(never)]
pub(super) fn casting_only(meshes: &mut Vec<(u64, Mat4, u32)>, drawn: &[&crate::world::Drawn]) {
    use crate::world::lighting::NO_CAST;
    meshes.clear();
    meshes.extend(
        drawn
            .iter()
            .filter(|d| d.material.color[3] >= 1.)
            .filter(|d| d.extras.is_none_or(|e| e.shadows & NO_CAST == 0))
            .map(|d| {
                let cutoff = d.surface.alpha_cutoff.map_or(0, f32::to_bits);
                (d.geometry.id(), d.matrix, cutoff)
            }),
    );
}

/// The casters' box along the sun, from a frame that drew the meshes
/// again (`cascades` keeps it for frames that don't). Out of the frame's
/// code, as `cascades` is.
#[inline(never)]
pub(super) fn cascade_scene(gpu: &mut Gpu, scene: (crate::math::Vec3, crate::math::Vec3)) {
    if let Some(sun) = &mut gpu.lights.sun {
        sun.scene = Some(scene);
    }
}

/// The sun's cascades this frame: each fit to its slice of the view, its
/// casters culled against it (where the meshes' `boxes` are current), and
/// which to draw again decided (`Sun::decide`), their frames and the main
/// pass's uniforms written. `changed` are the instances written this frame
/// (none where the meshes were kept), of `instances`. A world with a
/// cascade left waiting draws again (`Settle::CASCADES`). Their pipelines
/// made. Once a frame, out of its loop.
#[inline(never)]
pub(super) fn cascades(
    gpu: &mut Gpu,
    world: &Inner,
    (aspect, uniforms): (f32, &super::frame::FrameUniforms),
    (boxes, changed, instances): (
        Option<&super::cull::Boxes>,
        &[std::ops::Range<usize>],
        usize,
    ),
    runs: &[super::Run],
    (device, queue): (&wgpu::Device, &wgpu::Queue),
) {
    use super::shadows::{cascades as c, shadow_key};
    // Their casters' pipelines, made once.
    for run in runs.iter().filter(|run| !run.glass) {
        let key = shadow_key(run.variant);
        if gpu
            .lights
            .sun
            .as_ref()
            .is_some_and(|sun| !sun.pipelines.contains_key(&key))
        {
            let pipeline =
                super::shadows::caster_pipeline(gpu, device, run.variant, c::CASCADE_DEPTH);
            if let Some(sun) = &mut gpu.lights.sun {
                sun.pipelines.insert(key, pipeline);
            }
        }
    }
    let Some(sun) = &mut gpu.lights.sun else {
        return;
    };
    let [x, y, z, _] = uniforms.sun_direction;
    let direction = crate::vec3(x, y, z);
    let camera = &world.camera;
    let eye = c::Eye::new(camera, aspect);
    let count = sun.count();
    let settings = sun.settings;
    let splits = c::splits(
        camera.near,
        settings.distance.min(camera.far),
        count,
        settings.split,
    );
    let frame = c::sun_frame(direction);
    let nothing = (
        crate::math::Vec3::from([f32::MAX; 3]),
        crate::math::Vec3::from([f32::MIN; 3]),
    );
    let scene = sun.scene.unwrap_or(nothing);
    let mut fits = [c::Fit {
        matrix: Mat4::IDENTITY,
        texel: 0.,
    }; c::MAX_CASCADES];
    for (i, fit) in fits.iter_mut().enumerate().take(count) {
        *fit = c::fit(&eye, (splits[i], splits[i + 1]), &frame, scene, sun.side);
    }
    // A camera that sees nothing (at its own target, say) fits no view:
    // the maps hold what they did, moved by nothing, so the world rests.
    if !fits[..count].iter().all(c::Fit::usable) {
        let held: Option<Vec<c::Fit>> = sun.held[..count]
            .iter()
            .map(|held| held.as_ref().map(|held| held.fit))
            .collect();
        let Some(held) = held else {
            sun.redraw = [false; c::MAX_CASCADES];
            world.unsettle(crate::world::Settle::CASCADES, false);
            return;
        };
        fits[..count].copy_from_slice(&held);
    }
    // The lens the fits follow besides the camera's place: a change to it
    // fits every cascade again at once (it isn't the camera moving).
    let lens = Some([
        aspect.to_bits(),
        camera.fov.to_bits(),
        camera.near.to_bits(),
        camera.far.to_bits(),
        u32::from(camera.orthographic),
    ]);
    let lens_changed = sun.lens.is_some() && sun.lens != lens;
    sun.lens = lens;
    sun.track(device, changed, instances);
    let frame = c::Frame {
        fits: &fits[..count],
        sun: direction,
        changed,
        instances,
        masked: boxes.is_some(),
        lens: lens_changed,
    };
    // Which take the fit wanted now; each one's casters culled against the
    // fit it's drawn with.
    sun.adopt(&frame);
    if let Some(boxes) = boxes {
        for i in 0..count {
            let frustum = super::cull::Frustum::new(&sun.fit(i, &fits[..count]).matrix);
            super::cull::visible(&frustum, boxes, &mut sun.masks[i]);
        }
    }
    let left = sun.decide(&frame);
    world.unsettle(crate::world::Settle::CASCADES, left);
    let mut drawn = 0;
    for i in 0..count {
        if !sun.redraw[i] {
            continue;
        }
        let Some(fit) = sun.held[i].as_ref().map(|held| held.fit) else {
            continue;
        };
        drawn += 1;
        let geometries = &gpu.geometries;
        let indices = |run: &super::Run| geometries[&run.geometry].count;
        let masked = boxes.is_some();
        let mut spans = std::mem::take(&mut sun.spans[i]);
        let mut movers = std::mem::take(&mut sun.mover_spans[i]);
        spans.clear();
        movers.clear();
        match sun.how[i] {
            c::Draw::Whole => {
                let mask = masked.then(|| &sun.masks[i][..]);
                super::cull::shown(runs, mask, &mut spans, indices);
            }
            how => {
                // The still casters exactly (a mover in a gap drawn into
                // their layer would stay where it was); the movers' gaps
                // only draw still ones again.
                if how == c::Draw::Both {
                    let still = sun.still_casters(i, masked);
                    super::cull::shown(runs, Some(still), &mut spans, |_| u32::MAX);
                }
                listed_spans(runs, sun.out_of_layer(i, masked), &mut movers);
            }
        }
        let count = |spans: &[(u32, std::ops::Range<u32>)]| {
            spans.iter().map(|(_, s)| s.len() as u64).sum::<u64>()
        };
        sun.drawn += count(&spans) + count(&movers);
        sun.spans[i] = spans;
        sun.mover_spans[i] = movers;
        // Its caster pass's frame: today's map's, its view the cascade's.
        let mut frame: super::frame::FrameUniforms = bytemuck::Zeroable::zeroed();
        frame.shadow = fit.matrix.columns;
        queue.write_buffer(&sun.casters[i].0, 0, bytemuck::bytes_of(&frame));
    }
    if drawn > 0 {
        sun.draws += drawn;
        let out = sun.uniforms();
        queue.write_buffer(&sun.uniforms, 0, bytemuck::bytes_of(&out));
    }
}

/// Spans `(run, instances)` of the instances `listed` (in order), as
/// `cull::shown` gives them: each run's found by its instances' range.
fn listed_spans(runs: &[super::Run], listed: &[u32], out: &mut Vec<(u32, std::ops::Range<u32>)>) {
    let mut r = 0;
    for &i in listed {
        if !runs.get(r).is_some_and(|run| run.instances.contains(&i)) {
            r = runs.partition_point(|run| run.instances.end <= i);
            if r >= runs.len() || !runs[r].instances.contains(&i) {
                continue;
            }
        }
        match out.last_mut() {
            Some((at, span)) if *at == r as u32 && span.end == i => span.end = i + 1,
            _ => out.push((r as u32, i..i + 1)),
        }
    }
}

/// The cascades this frame draws again, each its casters' depth into its
/// layer. Out of the frame's code: inlined there, a frame of a world
/// without cascades cost 27 K instructions more (`mixed`).
#[inline(never)]
pub(super) fn draw_cascades(encoder: &mut wgpu::CommandEncoder, gpu: &Gpu, runs: &[super::Run]) {
    let (Some(sun), Some((instances, _))) = (&gpu.lights.sun, &gpu.instances) else {
        return;
    };
    use super::shadows::cascades::Draw;
    for c in 0..sun.count() {
        if !sun.redraw[c] {
            continue;
        }
        let how = sun.how[c];
        let still = sun.still.as_ref().filter(|_| how != Draw::Whole);
        // Its still casters into their layer.
        if let Some(still) = still.filter(|_| how == Draw::Both) {
            let mut pass = depth_pass(encoder, &still.layers[c]);
            pass.set_bind_group(0, &sun.casters[c].1, &[]);
            pass.set_vertex_buffer(1, instances.slice(..));
            super::shadows::draw_casters(&mut pass, gpu, runs, &sun.spans[c], &sun.pipelines);
        }
        let mut pass = depth_pass(encoder, &sun.layers[c]);
        let spans = match still {
            // That layer, then its movers.
            Some(still) => {
                pass.set_pipeline(&still.pipeline);
                pass.set_bind_group(0, &still.binds[c], &[]);
                pass.draw(0..3, 0..1);
                &sun.mover_spans[c]
            }
            None => &sun.spans[c],
        };
        pass.set_bind_group(0, &sun.casters[c].1, &[]);
        pass.set_vertex_buffer(1, instances.slice(..));
        super::shadows::draw_casters(&mut pass, gpu, runs, spans, &sun.pipelines);
    }
}

/// A pass drawing depth alone into `view`, cleared.
fn depth_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D sun cascade"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    })
}

#[cold]
#[inline(never)]
fn make_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D light clusters"),
        size: capacity,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A frame's layout and binding: lit by an environment (1), shading
/// through clusters (2), shadowed by cascades (4), lit by irradiance
/// probes as well (8, the `gi` feature's `probes`). 0 is today's own.
pub(super) const LIT: usize = 1;
pub(super) const CLUSTERED: usize = 2;
pub(super) const CASCADED: usize = 4;
pub(super) const PROBED: usize = 8;
/// `on`'s bit of a key.
fn bit(on: bool, bit: usize) -> usize {
    if on { bit } else { 0 }
}
/// `variant`'s frame's key.
pub(super) fn key_of(variant: super::Variant) -> usize {
    bit(variant.lit(), LIT)
        | bit(variant.clustered(), CLUSTERED)
        | bit(variant.cascaded(), CASCADED)
        | bit(variant.probed(), PROBED)
}

impl Lights {
    /// This frame's key but for its environment: clustered, cascaded.
    pub(super) fn key(&self) -> usize {
        bit(self.on, CLUSTERED) | bit(self.sun.is_some(), CASCADED)
    }
    /// The frame's binding where it shades through clusters or cascades.
    pub(super) fn frame_bind(&self, lit: bool, probed: bool) -> Option<&wgpu::BindGroup> {
        let key = self.key();
        if key == 0 {
            return None;
        }
        self.binds[key | bit(lit, LIT) | bit(probed, PROBED)]
            .as_ref()
            .map(|(bind, ..)| bind)
    }
    /// The window rests: what only a moving caster needs goes (the
    /// cascades' layers of still casters).
    pub(super) fn rest(&mut self) {
        if let Some(sun) = &mut self.sun {
            sun.rest();
        }
    }
    /// The bytes held for a moving caster alone (`rest` lets them go).
    pub(super) fn temporary_bytes(&self) -> u64 {
        self.sun.as_ref().map_or(0, |sun| sun.still_bytes())
    }
    /// What its bindings hold besides its own was made again (the probes'
    /// atlases): they're made again with the next frame.
    #[cfg_attr(not(feature = "gi"), allow(dead_code))]
    pub(super) fn remade(&mut self) {
        self.made += 1;
    }
}

/// The frame's binding where it shades through clusters or cascades, made
/// again where the renderer's frame binding (its shadow map, its
/// environment), the lists' buffer or the cascades were. After the
/// frame's shadow map and environment are made.
#[inline(never)]
pub(super) fn bind(gpu: &mut Gpu, device: &wgpu::Device, lit: bool, probed: bool) {
    let key = gpu.lights.key();
    if key == 0 {
        return;
    }
    let key = key | bit(lit, LIT) | bit(probed, PROBED);
    let from = if lit {
        gpu.environment_bind.as_ref()
    } else {
        Some(&gpu.bind)
    };
    let made = gpu.lights.made;
    if let Some((_, made_from, at)) = &gpu.lights.binds[key]
        && made_from.as_ref() == from
        && *at == made
    {
        return;
    }
    let from = from.cloned();
    if let Some(bind) = make_bind(gpu, device, key) {
        gpu.lights.binds[key] = Some((bind, from, made));
    }
}

#[cold]
#[inline(never)]
fn make_bind(gpu: &mut Gpu, device: &wgpu::Device, key: usize) -> Option<wgpu::BindGroup> {
    make_layouts(gpu, device, key);
    let lights = &gpu.lights;
    let layout = lights.layouts[key].as_ref()?;
    let map = gpu.shadow_map.as_ref().unwrap_or(&gpu.stand_in);
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: gpu.uniforms.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(map),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(&gpu.sampler),
        },
    ];
    if key & LIT != 0 {
        let (Some((_, cube)), Some((_, sampler))) = (&gpu.environment, &gpu.environment_layout)
        else {
            return None;
        };
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::TextureView(cube),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Sampler(sampler),
        });
    }
    if key & CLUSTERED != 0 {
        let (buffer, _) = lights.buffer.as_ref()?;
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: buffer.as_entire_binding(),
        });
    }
    if key & CASCADED != 0 {
        let sun = lights.sun.as_ref()?;
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: sun.uniforms.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: wgpu::BindingResource::TextureView(&sun.array),
        });
    }
    #[cfg(feature = "gi")]
    if key & PROBED != 0 {
        entries.extend(super::probes::bind_entries(gpu)?);
    }
    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3D frame, clustered lights, cascades or probes"),
        layout,
        entries: &entries,
    }))
}

/// The frame's layout for a pipeline of `key` (`key_of`), which
/// `make_layouts` made.
pub(super) fn layout_of(gpu: &Gpu, key: usize) -> &wgpu::BindGroupLayout {
    gpu.lights.layouts[key]
        .as_ref()
        .expect("made with the pipeline")
}

/// The frame's layout of `key`, made once.
#[cold]
#[inline(never)]
pub(super) fn make_layouts(gpu: &mut Gpu, device: &wgpu::Device, key: usize) {
    // The environment's sampler, which a lit binding takes.
    if key & LIT != 0 && gpu.environment_layout.is_none() {
        gpu.environment_layout = Some(super::frame::environment_layout(device));
    }
    if gpu.lights.layouts[key].is_some() {
        return;
    }
    let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    };
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let depth = |dimension| wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Depth,
        view_dimension: dimension,
        multisampled: false,
    };
    let mut entries = vec![
        entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, uniform),
        entry(1, fragment, depth(wgpu::TextureViewDimension::D2)),
        entry(
            2,
            fragment,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        ),
    ];
    if key & LIT != 0 {
        entries.push(entry(
            3,
            fragment,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::Cube,
                multisampled: false,
            },
        ));
        entries.push(entry(
            4,
            fragment,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        ));
    }
    if key & CLUSTERED != 0 {
        entries.push(entry(
            5,
            fragment,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ));
    }
    if key & CASCADED != 0 {
        entries.push(entry(6, fragment, uniform));
        entries.push(entry(
            7,
            fragment,
            depth(wgpu::TextureViewDimension::D2Array),
        ));
    }
    #[cfg(feature = "gi")]
    if key & PROBED != 0 {
        entries.extend(super::probes::layout_entries());
    }
    gpu.lights.layouts[key] = Some(device.create_bind_group_layout(
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("3D frame, clustered lights, cascades or probes"),
            entries: &entries,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::super::testing::device_and_queue;
    use super::*;
    use crate::{Camera, Geometry, Material, Object, World, vec3};
    use zgx_gpu::viewport::{Viewport, ViewportFrame};

    fn draw(renderer: &mut super::super::Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
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
    }

    fn bulb(x: f32) -> Light {
        Light::Point {
            position: vec3(x, 1., 0.),
            color: [1.; 3],
            intensity: 1.,
            range: 3.,
        }
    }

    #[test]
    fn lights_are_binned_only_when_a_light_or_the_camera_changed() {
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(20., 20.), Material::default()));
        let lights: Vec<_> = (0..12).map(|i| world.add_light(bulb(i as f32))).collect();
        let mut renderer = world.renderer();
        let binnings = |r: &super::super::Renderer| r.gpu.as_ref().unwrap().lights.binnings;
        draw(&mut renderer, &device, &queue);
        assert_eq!(binnings(&renderer), 1);
        // Drawn again with nothing changed (a resize of the box elsewhere,
        // say): the lists stay.
        draw(&mut renderer, &device, &queue);
        assert_eq!(binnings(&renderer), 1);
        world.set_background(Some(0x102030));
        draw(&mut renderer, &device, &queue);
        assert_eq!(binnings(&renderer), 1, "the background moves no light");
        world.set_light(lights[3], bulb(-4.));
        draw(&mut renderer, &device, &queue);
        assert_eq!(binnings(&renderer), 2, "a light moved");
        world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., 0.]));
        draw(&mut renderer, &device, &queue);
        assert_eq!(binnings(&renderer), 3, "the camera moved");
        // Down to eight: the uniform path again, nothing binned.
        for &light in &lights[8..] {
            world.remove_light(light);
        }
        draw(&mut renderer, &device, &queue);
        assert!(!renderer.gpu.as_ref().unwrap().lights.on);
        assert_eq!(binnings(&renderer), 3);
    }

    #[test]
    fn a_clustered_shader_reads_its_lights_from_the_clusters_alone() {
        use super::super::{Variant, shader::wgsl};
        let plain = wgsl(Variant::default(), 4);
        let clustered = wgsl(Variant::default().with(Variant::CLUSTERED, true), 4);
        assert!(!plain.contains("clusters") && plain.contains("frame.points[i]"));
        assert!(clustered.contains("var<storage, read> clusters: array<vec4<u32>>"));
        assert!(clustered.contains("let lamp = lamp_at(cluster_light(i), in.world);"));
        assert!(!clustered.contains("frame.points[i]"));
    }

    #[test]
    fn eight_lights_or_fewer_take_the_uniform_path_unless_clusters_are_asked_for() {
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(20., 20.), Material::default()));
        for i in 0..8 {
            world.add_light(bulb(i as f32));
        }
        let mut renderer = world.renderer();
        draw(&mut renderer, &device, &queue);
        let gpu = renderer.gpu.as_ref().unwrap();
        assert!(
            !gpu.lights.on
                && gpu.lights.layouts.iter().all(Option::is_none)
                && gpu.lights.buffer.is_none()
        );
        world.set_clustered_lights(true);
        draw(&mut renderer, &device, &queue);
        assert!(renderer.gpu.as_ref().unwrap().lights.on);
    }

    #[test]
    fn cascades_are_drawn_again_only_as_they_move_the_far_ones_one_a_frame() {
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(40., 40.), Material::default()));
        let cube = world.add(Object::mesh(
            Geometry::cuboid(1., 1., 1.),
            Material::default(),
        ));
        world.add_light(Light::Directional {
            direction: vec3(-0.4, -1., -0.3),
            color: [1.; 3],
            intensity: 2.,
        });
        world.set_shadows(true);
        world.set_cascades(Some(crate::Cascades::new(4).distance(40.).size(256)));
        world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., 0.]));
        let mut renderer = world.renderer();
        let draws = |r: &super::super::Renderer| {
            let sun = r.gpu.as_ref().unwrap().lights.sun.as_ref().unwrap();
            (sun.draws, sun.redraw)
        };
        draw(&mut renderer, &device, &queue);
        assert_eq!(draws(&renderer), (4, [true; 4]), "all, the first frame");
        assert!(world.inner().settle.settled());
        draw(&mut renderer, &device, &queue);
        assert_eq!(draws(&renderer).0, 4, "none: nothing moved");
        // The camera moves far enough to move every cascade: the nearest
        // and one far one now, the other two in the frames after, the
        // world drawing again until they have.
        world.set_camera(Camera::looking_at([3., 5., 12.], [1., 0., 0.]));
        draw(&mut renderer, &device, &queue);
        let (count, redraw) = draws(&renderer);
        assert_eq!(count, 6);
        assert!(redraw[0] && redraw.iter().filter(|r| **r).count() == 2);
        assert!(
            !world.inner().settle.settled(),
            "waiting cascades keep it drawing"
        );
        draw(&mut renderer, &device, &queue);
        draw(&mut renderer, &device, &queue);
        assert_eq!(draws(&renderer).0, 8);
        assert!(world.inner().settle.settled());
        draw(&mut renderer, &device, &queue);
        assert_eq!(draws(&renderer).0, 8, "caught up");
        // The cube, 13 m from the camera (where the third cascade meets the
        // fourth), moves: the cascades holding it, at once, not the two
        // nearer.
        world.set_transform(cube, crate::Transform::at([0.2, 0., 0.]));
        draw(&mut renderer, &device, &queue);
        assert_eq!(draws(&renderer), (10, [false, false, true, true]));
        // Off: today's single map again, the cascades gone.
        world.set_cascades(None);
        draw(&mut renderer, &device, &queue);
        let gpu = renderer.gpu.as_ref().unwrap();
        assert!(gpu.lights.sun.is_none() && gpu.shadow_map.is_some());
    }

    #[test]
    fn a_caster_moving_among_still_ones_redraws_only_itself_into_the_cascades() {
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(40., 40.), Material::default()));
        // 48 still cubes about the view, and one moving among them.
        for i in 0..48 {
            let at = [(i % 8) as f32 * 2. - 7., 0.5, (i / 8) as f32 * -3. + 4.];
            world.add(
                Object::mesh(Geometry::cuboid(1., 1., 1.), Material::default())
                    .transformed(crate::Transform::at(at)),
            );
        }
        let mover = world.add(Object::mesh(
            Geometry::cuboid(1., 1., 1.),
            Material::default(),
        ));
        world.add_light(Light::Directional {
            direction: vec3(-0.4, -1., -0.3),
            color: [1.; 3],
            intensity: 2.,
        });
        world.set_shadows(true);
        world.set_cascades(Some(crate::Cascades::new(4).distance(40.).size(256)));
        world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., -4.]));
        let mut renderer = world.renderer();
        let drawn = |r: &super::super::Renderer| {
            let sun = r.gpu.as_ref().unwrap().lights.sun.as_ref().unwrap();
            sun.drawn
        };
        let step = |renderer: &mut super::super::Renderer, frame: usize| {
            let x = (frame as f32 * 0.1).sin() * 3.;
            world.set_transform(mover, crate::Transform::at([x, 1.5, 0.]));
            draw(renderer, &device, &queue);
        };
        // Long enough that the still ones count as still.
        for frame in 0..40 {
            step(&mut renderer, frame);
        }
        let before = drawn(&renderer);
        for frame in 40..50 {
            step(&mut renderer, frame);
        }
        // The mover alone, into each cascade holding it: the still cubes
        // and the plane stay in each's layer of still casters.
        let per_frame = (drawn(&renderer) - before) as f64 / 10.;
        assert!(per_frame <= 4., "{per_frame} instances a frame");
        // The camera moving too: the nearest cascade, fit again each frame,
        // is drawn whole, its still casters' layer not drawn for nothing.
        for frame in 50..55 {
            let x = (frame - 49) as f32 * 0.3;
            // Close to the mover: the nearest cascade holds it.
            world.set_camera(Camera::looking_at([x, 3., 2.5], [0., 1.5, 0.]));
            step(&mut renderer, frame);
            let sun = renderer.gpu.as_ref().unwrap().lights.sun.as_ref().unwrap();
            assert!(sun.redraw[0], "frame {frame}");
            assert_eq!(sun.how[0], super::super::shadows::cascades::Draw::Whole);
        }
        // The camera turned sharply away: of the far cascades holding the
        // mover, those not taking the camera's fit this frame keep theirs,
        // and draw the mover there, culled against it.
        world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., -4.]));
        for frame in 55..60 {
            step(&mut renderer, frame);
        }
        let sun = |r: &super::super::Renderer| {
            let sun = r.gpu.as_ref().unwrap().lights.sun.as_ref().unwrap();
            let fits: Vec<_> = sun.held.iter().map(|h| h.as_ref().map(|h| h.fit)).collect();
            (fits, sun.mover_spans.clone())
        };
        let (fits, movers) = sun(&renderer);
        let holding: Vec<usize> = (2..4).filter(|&c| !movers[c].is_empty()).collect();
        assert!(!holding.is_empty(), "a far cascade holds the mover");
        world.set_camera(Camera::looking_at([0., 4., 9.], [30., 0., 9.]));
        step(&mut renderer, 60);
        let (after, movers) = sun(&renderer);
        assert_ne!(fits[0], after[0], "the nearest follows the camera");
        for c in holding {
            assert_eq!(fits[c], after[c], "cascade {c} keeps its fit");
            assert!(!movers[c].is_empty(), "cascade {c} draws the mover");
        }
    }

    #[test]
    fn listed_instances_are_spans_of_their_runs() {
        use super::super::Run;
        let run = |geometry, instances| Run {
            geometry,
            glass: false,
            maps: [0; crate::texture::MAPS],
            variant: Default::default(),
            instances,
            casts: true,
        };
        let runs = [run(0, 0..4), run(1, 4..12), run(2, 12..14)];
        let mut out = Vec::new();
        // Side by side within a run: one span; across runs' edge: one each.
        super::listed_spans(&runs, &[2, 3, 4, 5, 9, 13], &mut out);
        assert_eq!(out, [(0, 2..4), (1, 4..6), (1, 9..10), (2, 13..14)]);
        // Past the last run: left out.
        out.clear();
        super::listed_spans(&runs, &[0, 14], &mut out);
        assert_eq!(out, [(0, 0..1)]);
    }

    #[test]
    fn the_still_casters_layers_go_once_nothing_moves_or_the_window_rests() {
        use zgx_gpu::viewport::Viewport;
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(40., 40.), Material::default()));
        let mover = world.add(Object::mesh(
            Geometry::cuboid(1., 1., 1.),
            Material::default(),
        ));
        world.add_light(Light::Directional {
            direction: vec3(-0.4, -1., -0.3),
            color: [1.; 3],
            intensity: 2.,
        });
        world.set_shadows(true);
        world.set_cascades(Some(crate::Cascades::new(4).distance(40.).size(256)));
        world.set_camera(Camera::looking_at([0., 4., 9.], [0., 0., -4.]));
        let mut renderer = world.renderer();
        let held = |r: &super::super::Renderer| {
            let sun = r.gpu.as_ref().unwrap().lights.sun.as_ref().unwrap();
            sun.still_bytes()
        };
        let step = |renderer: &mut super::super::Renderer, x: f32| {
            world.set_transform(mover, crate::Transform::at([x, 0.5, 0.]));
            draw(renderer, &device, &queue);
        };
        draw(&mut renderer, &device, &queue);
        assert_eq!(held(&renderer), 0, "nothing moved: none made");
        for frame in 1..=5 {
            step(&mut renderer, frame as f32 * 0.1);
        }
        // Four of 256 square, 2 bytes a texel, counted with the frame's.
        let layers = 4 * 256 * 256 * 2;
        assert_eq!(held(&renderer), layers);
        let before = world.temporary_bytes();
        // Stopped: drawn on (a camera turning, say), they go as it turns
        // still, `MOVING` frames after its last change.
        for frame in 1..super::super::shadows::cascades::MOVING {
            world.set_background(Some(frame as u32));
            draw(&mut renderer, &device, &queue);
            assert_eq!(held(&renderer), layers, "frame {frame}: still moving");
        }
        world.set_background(None);
        draw(&mut renderer, &device, &queue);
        assert_eq!(held(&renderer), 0, "none moves");
        assert_eq!(world.temporary_bytes(), before - layers);
        // Moving again, then the window rests: they go with its other
        // temporaries, whatever moves.
        step(&mut renderer, 2.);
        step(&mut renderer, 2.1);
        assert_eq!(held(&renderer), layers);
        renderer.rest();
        assert_eq!(held(&renderer), 0, "rested");
        assert_eq!(world.temporary_bytes(), 0);
    }

    #[test]
    fn lights_that_shine_nothing_are_neither_gathered_nor_binned() {
        let (device, queue) = device_and_queue();
        let world = World::new();
        world.add(Object::mesh(Geometry::plane(20., 20.), Material::default()));
        // 300 lamps off (no lumens, or black) and one on.
        for i in 0..300 {
            let at = vec3((i % 20) as f32 - 10., 1., (i / 20) as f32 - 7.);
            let lamp = crate::Light::point(at).range(3.);
            world.add_point_light(if i % 2 == 0 {
                lamp.lumens(0.)
            } else {
                lamp.color([0.; 3])
            });
        }
        world.add_light(bulb(0.));
        let mut renderer = world.renderer();
        draw(&mut renderer, &device, &queue);
        let lights = &renderer.gpu.as_ref().unwrap().lights;
        assert!(!lights.on, "one light shining: the uniform path");
        assert_eq!(lights.binnings, 0);
        // Clustered whatever, only the one shining is binned.
        world.set_clustered_lights(true);
        draw(&mut renderer, &device, &queue);
        let lights = &renderer.gpu.as_ref().unwrap().lights;
        assert_eq!(lights.binned.lights.len(), 1);
    }
}
