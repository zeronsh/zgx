//! Custom materials on the GPU (`World::add_shader`): their pipelines by
//! shader and variant, their bindings' own entries, the frame's tail (view,
//! projection, size, time and the surface layers' numbers) and each
//! shader's numbers, written as they change. A world with no shader and no
//! layer runs none of it: `prepare` returns at its first line.
use super::{
    Gpu, Run, Variant,
    frame::FrameUniforms,
    materials::{Extent, map_layout_entries, upload},
    pipelines::{extent_of, main_pipelines, make_main_pipelines, pipelines_of},
    shader::wgsl_with,
    shadows::{make_shadow_pipeline, shadow_key, shadow_pipeline},
};
use crate::{
    material::{
        Custom, FRAME_TAIL, Param, SAMPLER_BINDING, SCENE_BINDING, Shader, TEXTURE_BINDING,
        UNIFORM_BINDING, layer_offsets,
    },
    texture::MAPS,
    world::Inner,
};
use std::{collections::HashMap, rc::Rc, time::Instant};
use zgx_gpu::{viewport::ViewportFrame, wgpu};

/// The `Settle` reason of a mesh in view whose shader reads time.
pub(crate) const ANIMATED: u32 = 1 << 1;

/// What a renderer keeps of custom materials.
#[derive(Default)]
pub(super) struct State {
    /// Pipelines by shader and variant, made with the layers as they are
    /// (`layers`).
    pipelines: HashMap<(u32, Variant), [wgpu::RenderPipeline; 2]>,
    /// Shadows' pipelines by shader and variant, for shaders that move
    /// (`displace`) or cut out (`opacity`) their meshes.
    shadows: HashMap<(u32, Variant), wgpu::RenderPipeline>,
    /// Each custom binding's shader, by its key (`map_set`).
    by_binding: HashMap<[u64; MAPS], Rc<Shader>>,
    /// Group 1's layout by shader and how much else it binds.
    layouts: HashMap<(u32, Extent), wgpu::BindGroupLayout>,
    /// Each shader's numbers, and the count of their changes they hold.
    uniforms: HashMap<u32, (wgpu::Buffer, u64)>,
    /// Pipelines the device refused, by shader, variant and whether its
    /// shadows': not made again.
    failed: std::collections::HashSet<(u32, Variant, bool)>,
    /// Shaders' textures, uploaded, by id and whether read as sRGB.
    views: HashMap<(u64, bool), wgpu::TextureView>,
    /// The surface layers the stock pipelines were made with, and the code
    /// they add.
    layers: Vec<Rc<Shader>>,
    layer_code: Option<Custom>,
    /// Whether a run of the frame's reads what is behind (`prepare`).
    reads: bool,
    /// Which custom runs the shadow map was last drawn with (a number):
    /// it holds what their shaders made of them.
    casting: u64,
    /// When the first frame was drawn, and the last; frames drawn.
    epoch: Option<Instant>,
    last: Option<Instant>,
    index: u32,
}
impl State {
    /// The code every stock pipeline compiles with: the surface layers'.
    pub(super) fn layers(&self) -> Option<&Custom> {
        self.layer_code.as_ref()
    }
}

/// What a custom binding takes from the renderer: its sampler, white
/// stand-ins (sRGB, linear) and what is behind.
pub(super) struct Own<'a> {
    pub(super) sampler: &'a wgpu::Sampler,
    pub(super) white: [&'a wgpu::TextureView; 2],
    pub(super) behind: Option<(&'a wgpu::TextureView, &'a wgpu::Sampler)>,
}
enum Resource {
    Buffer(wgpu::Buffer),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}
/// A custom binding's layout, and what it binds past a mesh's own maps.
pub(super) struct Binding {
    pub(super) layout: wgpu::BindGroupLayout,
    resources: Vec<(u32, Resource)>,
}
impl Binding {
    pub(super) fn entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        self.resources
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match resource {
                    Resource::Buffer(buffer) => buffer.as_entire_binding(),
                    Resource::View(view) => wgpu::BindingResource::TextureView(view),
                    Resource::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                },
            })
            .collect()
    }
}

/// The binding of `shader`'s meshes past their maps (as far as `extent`),
/// keyed `key`: its layout made, its uniform made and written, its textures
/// uploaded. Made with a binding: rarely.
#[cold]
#[inline(never)]
pub(super) fn binding_of(
    state: &mut State,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shader: &crate::world::ShaderRef,
    extent: Extent,
    own: Own<'_>,
    key: [u64; MAPS],
) -> Binding {
    let shader = &shader.0;
    state.by_binding.insert(key, shader.clone());
    let layout = state
        .layouts
        .entry((shader.id, extent))
        .or_insert_with(|| layout_of(device, shader, extent))
        .clone();
    let mut resources = vec![(SAMPLER_BINDING, Resource::Sampler(own.sampler.clone()))];
    if shader.uniform_size > 0 {
        let (buffer, _) = uniform_of(state, device, queue, shader);
        resources.push((UNIFORM_BINDING, Resource::Buffer(buffer.clone())));
    }
    let values = shader.values.borrow();
    let largest = device.limits().max_texture_dimension_2d;
    for slot in shader.slots.iter() {
        let srgb = match slot.kind {
            Param::Texture => true,
            Param::DataTexture => false,
            _ => continue,
        };
        // A texture past what the device holds binds as none (and says so).
        let texture = values.textures[slot.at as usize].as_ref().filter(|t| {
            let fits = t.width().max(t.height()) <= largest;
            if !fits {
                shader.failed.replace(Some(format!(
                    "`{}` is {}x{}: this device's textures are {largest} at most",
                    slot.name,
                    t.width(),
                    t.height()
                )));
            }
            fits
        });
        let view = texture
            .and_then(|texture| {
                use std::collections::hash_map::Entry;
                match state.views.entry((texture.id(), srgb)) {
                    Entry::Occupied(held) => Some(held.get().clone()),
                    Entry::Vacant(free) => {
                        Some(free.insert(upload(device, queue, texture, srgb)?).clone())
                    }
                }
            })
            .unwrap_or_else(|| own.white[usize::from(!srgb)].clone());
        resources.push((TEXTURE_BINDING + slot.at, Resource::View(view)));
    }
    if shader.reads_scene {
        let (view, sampler) = own.behind.unwrap_or((own.white[1], own.sampler));
        resources.push((SCENE_BINDING, Resource::View(view.clone())));
        resources.push((SCENE_BINDING + 1, Resource::Sampler(sampler.clone())));
    }
    Binding { layout, resources }
}
/// What the device refused of a binding of `shader`'s, keyed `key`: its
/// mesh binds as without it and draws by the stock pipelines.
#[cold]
#[inline(never)]
pub(super) fn refused_binding(
    state: &mut State,
    shader: &crate::world::ShaderRef,
    key: [u64; MAPS],
    error: String,
) {
    state.by_binding.remove(&key);
    shader.0.failed.replace(Some(error));
}
/// `shader`'s uniform, made and written with its numbers if new.
fn uniform_of<'a>(
    state: &'a mut State,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shader: &Shader,
) -> &'a (wgpu::Buffer, u64) {
    state.uniforms.entry(shader.id).or_insert_with(|| {
        let values = shader.values.borrow();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D material parameters"),
            size: u64::from(shader.uniform_size),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, &values.bytes);
        (buffer, values.numbers)
    })
}
/// Group 1's layout for `shader` as far as `extent`: a mesh's maps and the
/// rest it binds, then its shader's sampler, numbers, textures and what is
/// behind.
fn layout_of(device: &wgpu::Device, shader: &Shader, extent: Extent) -> wgpu::BindGroupLayout {
    let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: both,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: both,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let mut entries = map_layout_entries(extent);
    entries.push(sampler(SAMPLER_BINDING));
    if shader.uniform_size > 0 {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: UNIFORM_BINDING,
            visibility: both,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
    }
    entries.extend((0..shader.textures).map(|i| texture(TEXTURE_BINDING + i)));
    if shader.reads_scene {
        entries.extend([texture(SCENE_BINDING), sampler(SCENE_BINDING + 1)]);
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("3D custom material"),
        entries: &entries,
    })
}

/// A custom run's pipelines, if made: its binding's shader's, for its
/// variant. (Out of `draw_run`'s code: few runs are custom.)
#[cold]
#[inline(never)]
pub(super) fn pipelines<'a>(gpu: &'a Gpu, run: &Run) -> Option<&'a [wgpu::RenderPipeline; 2]> {
    let shader = gpu.custom.by_binding.get(&run.maps)?;
    gpu.custom.pipelines.get(&(shader.id, run.variant))
}
/// `run`'s shadow, a custom material's: by its shader's own pipeline if
/// it moves, cuts out or is cut out (`make_shadow`), else by the stock one
/// of its variant without its shader, which binds none of its maps. Whether
/// it was drawn.
#[cold]
#[inline(never)]
pub(super) fn draw_shadow(pass: &mut wgpu::RenderPass<'_>, gpu: &Gpu, run: &Run) -> bool {
    let Some(shader) = gpu.custom.by_binding.get(&run.maps) else {
        return false;
    };
    let own = gpu.custom.shadows.get(&(shader.id, run.variant));
    let stock = shadow_key(whole(run.variant));
    let pipeline = match own {
        Some(pipeline) => pipeline,
        None if stock == 0 => &gpu.shadow,
        None => match gpu.shadow_variants.get(&stock) {
            Some(pipeline) => pipeline,
            None => return false,
        },
    };
    let buffers = &gpu.geometries[&run.geometry];
    let (skinned, morphing) = (run.variant.skinned(), run.variant.morphing());
    pass.set_pipeline(pipeline);
    if own.is_some() {
        let Some((binding, _)) = gpu.bindings.get(&run.maps) else {
            return false;
        };
        pass.set_bind_group(1, binding, &[]);
    } else if stock != 0 {
        pass.set_bind_group(1, &gpu.empty_bind, &[]);
    }
    if let Some((_, _, joints)) = gpu.palettes.as_ref().filter(|_| skinned || morphing) {
        pass.set_bind_group(2, joints, &[]);
    }
    if let Some(skin) = buffers.skin.as_ref().filter(|_| skinned) {
        pass.set_vertex_buffer(2, skin.slice(..));
    }
    if let Some(morphs) = buffers.morphs.as_ref().filter(|_| morphing) {
        pass.set_bind_group(3, morphs, &[]);
    }
    pass.set_vertex_buffer(0, buffers.vertices.slice(..));
    pass.set_index_buffer(buffers.indices.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..buffers.count, 0, run.instances.clone());
    true
}
/// `variant` without its custom material or its cut: what its mesh casts
/// by where its shader's own pipeline doesn't (it binds none of its maps).
fn whole(variant: Variant) -> Variant {
    variant
        .with(Variant::CUSTOM, false)
        .with(Variant::MASKED, false)
        .with(Variant::PLACED, false)
}
/// Whether a shader of the world's moves its meshes' vertices: their
/// boxes are then made each frame its meshes change (`grow_boxes`).
pub(super) fn boxes_needed(world: &Inner) -> bool {
    world.shaders.displacing()
}
/// The boxes of meshes whose shader moves their vertices: grown by as far
/// as it says (`MaterialShader::displaces_by`), else not sure (never
/// culled). Made again from each mesh's own box, so frames don't grow
/// them twice. (Out of the frame's code: only custom runs, in a world with
/// a displacing shader.)
#[cold]
#[inline(never)]
pub(super) fn grow_boxes(gpu: &Gpu, scratch: &mut super::Scratch, drawn: &[&crate::world::Drawn]) {
    for run in scratch.runs.iter().filter(|run| run.variant.custom()) {
        let Some(shader) = gpu.custom.by_binding.get(&run.maps).filter(|s| s.displace) else {
            continue;
        };
        let range = run.instances.start as usize..run.instances.end as usize;
        for (at, d) in drawn.iter().enumerate().take(range.end).skip(range.start) {
            let (low, high) = super::cull::world_box(d);
            let (grown, sure) = match shader.reach {
                Some(r) => (
                    (low - crate::vec3(r, r, r), high + crate::vec3(r, r, r)),
                    true,
                ),
                None => ((low, high), false),
            };
            // (A posed mesh not drawn posed isn't sure either: as it was.)
            let sure = sure && scratch.boxes.sure(at);
            scratch.boxes.set(at, grown, sure);
        }
    }
}
/// Whether a run of the frame's custom materials reads what is behind it.
pub(super) fn reads(gpu: &Gpu) -> bool {
    gpu.custom.reads
}
/// Whether `run`'s shader reads what is behind: kept out of what is behind.
#[cold]
#[inline(never)]
pub(super) fn reads_scene(gpu: &Gpu, run: &Run) -> bool {
    gpu.custom
        .by_binding
        .get(&run.maps)
        .is_some_and(|shader| shader.reads_scene)
}

/// What a frame with custom materials needs before its passes: the stock
/// pipelines made again if the surface layers changed, the frame's tail and
/// shaders' numbers written, custom runs' pipelines made, and the world kept
/// drawing while a mesh in view reads time, and whether a run reads what
/// is behind (`reads`).
#[inline(never)]
pub(super) fn prepare(
    gpu: &mut Gpu,
    world: &Inner,
    frame: &ViewportFrame<'_>,
    (runs, shown): (&[Run], &[(u32, std::ops::Range<u32>)]),
) {
    let shaders = &world.shaders;
    if shaders.is_empty() && gpu.custom.layers.is_empty() {
        gpu.custom.reads = false;
        return;
    }
    let device = frame.device;
    if !same(&gpu.custom.layers, &shaders.layers) {
        relayer(gpu, device, &shaders.layers, runs);
    }
    write_tail(gpu, world, frame);
    let mut reads = false;
    let mut casting = 0xcbf2_9ce4_8422_2325_u64;
    for run in runs.iter().filter(|run| run.variant.custom()) {
        let Some(shader) = gpu.custom.by_binding.get(&run.maps).cloned() else {
            continue;
        };
        reads |= shader.reads_scene;
        // It casts by a pipeline of its own if it moves or cuts out its
        // mesh, or its mesh is cut out (its binding is its shader's).
        let casts = shader.displace
            || shader.opacity
            || (run.variant.masked() && !run.variant.transmitting());
        if casts && !run.glass {
            for n in [shader.id, run.instances.start, run.instances.end].map(u64::from) {
                casting = (casting ^ n).wrapping_mul(0x0100_0000_01b3);
            }
        }
        if shader.uniform_size > 0 {
            let numbers = shader.values.borrow().numbers;
            let (buffer, held) = uniform_of(&mut gpu.custom, device, frame.queue, &shader);
            if *held != numbers {
                frame
                    .queue
                    .write_buffer(buffer, 0, &shader.values.borrow().bytes);
                gpu.custom.uniforms.get_mut(&shader.id).unwrap().1 = numbers;
                if casts {
                    // Its shadow moves with its numbers: drawn again.
                    gpu.shadow_drawn = None;
                }
            }
        }
        let failed = |gpu: &Gpu, shadow| {
            gpu.custom
                .failed
                .contains(&(shader.id, run.variant, shadow))
        };
        if !gpu.custom.pipelines.contains_key(&(shader.id, run.variant)) && !failed(gpu, false) {
            make_pipelines(gpu, device, &shader, run.variant);
        }
        if !run.glass && gpu.shadow_map.is_some() {
            let own = (shader.id, run.variant);
            if casts && !gpu.custom.shadows.contains_key(&own) && !failed(gpu, true) {
                make_shadow(gpu, device, &shader, run.variant);
            }
            if !gpu.custom.shadows.contains_key(&own) {
                // Else as its variant without it casts, whole.
                let stock = whole(run.variant);
                let key = shadow_key(stock);
                if key != 0 && !gpu.shadow_variants.contains_key(&key) {
                    make_shadow_pipeline(gpu, device, stock);
                }
            }
        }
    }
    // A layer reading time animates every lit mesh it runs over; a mesh's
    // shader, its own.
    let layered = shaders.layers.iter().any(|l| l.animated) && !shown.is_empty();
    let animated = layered
        || shaders.list.iter().any(|s| s.animated)
            && shown.chunk_by(|a, b| a.0 == b.0).any(|spans| {
                let run = &runs[spans[0].0 as usize];
                run.variant.custom()
                    && gpu
                        .custom
                        .by_binding
                        .get(&run.maps)
                        .is_some_and(|shader| shader.animated)
            });
    world.unsettle(ANIMATED, animated);
    // Meshes cast by other shaders than the map holds: drawn again.
    if std::mem::replace(&mut gpu.custom.casting, casting) != casting {
        gpu.shadow_drawn = None;
    }
    // Bindings gone, their shaders with them.
    let bindings = &gpu.bindings;
    gpu.custom
        .by_binding
        .retain(|key, _| bindings.contains_key(key));
    gpu.custom.reads = reads;
}
fn same(a: &[Rc<Shader>], b: &[Rc<Shader>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Rc::ptr_eq(a, b))
}
/// The stock pipelines made again with `layers`, and the custom ones
/// dropped to be made again.
#[cold]
#[inline(never)]
fn relayer(gpu: &mut Gpu, device: &wgpu::Device, layers: &[Rc<Shader>], runs: &[Run]) {
    let state = &mut gpu.custom;
    state.layers = layers.to_vec();
    state.layer_code = (!layers.is_empty()).then(|| Custom::new(None, layers.iter().map(|l| &**l)));
    state.pipelines.clear();
    state.shadows.clear();
    gpu.variants.clear();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("3D"),
        source: wgpu::ShaderSource::Wgsl(
            wgsl_with(
                Variant::default(),
                gpu.samples,
                gpu.custom.layer_code.as_ref(),
            )
            .into(),
        ),
    });
    gpu.plain = main_pipelines(
        device,
        (gpu.format, gpu.samples),
        &shader,
        &[Some(&gpu.layout)],
        Variant::default(),
    );
    for run in runs {
        let variant = run.variant;
        if variant != Variant::default() && !gpu.variants.contains_key(&variant) {
            make_main_pipelines(gpu, device, variant);
        }
    }
}
/// `shader`'s pipelines for `variant`.
#[cold]
#[inline(never)]
fn make_pipelines(gpu: &mut Gpu, device: &wgpu::Device, shader: &Rc<Shader>, variant: Variant) {
    let extent = extent_of(variant).unwrap_or(Extent::Maps);
    let layout = gpu
        .custom
        .layouts
        .entry((shader.id, extent))
        .or_insert_with(|| layout_of(device, shader, extent))
        .clone();
    super::pipelines::make_layouts(gpu, device, variant, Some(extent));
    let custom = Custom::new(Some(shader), gpu.custom.layers.iter().map(|l| &**l));
    let source = wgsl_with(variant, gpu.samples, Some(&custom));
    match scoped(device, || {
        pipelines_of(gpu, device, variant, source, &layout)
    }) {
        Ok(pipelines) => {
            gpu.custom.pipelines.insert((shader.id, variant), pipelines);
        }
        Err(error) => refused(gpu, shader, (variant, false), error),
    }
}
/// `shader`'s shadows' pipeline for `variant`'s meshes: moved by its
/// `displace`, cut out by its `opacity` (and its base colour's alpha, if
/// it's cut out by that too).
#[cold]
#[inline(never)]
fn make_shadow(gpu: &mut Gpu, device: &wgpu::Device, shader: &Rc<Shader>, variant: Variant) {
    let extent = extent_of(variant).unwrap_or(Extent::Maps);
    let layout = gpu
        .custom
        .layouts
        .entry((shader.id, extent))
        .or_insert_with(|| layout_of(device, shader, extent))
        .clone();
    let casting = Variant::default()
        .with(Variant::TEXTURED, true)
        .with(Variant::CUSTOM, true)
        .with(Variant::SKINNED, variant.skinned())
        .with(Variant::MORPHING, variant.morphing())
        .with(Variant::MASKED, shader.opacity || variant.masked());
    let custom = Custom::new(Some(shader), std::iter::empty());
    let made = scoped(device, || {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D custom material's shadows"),
            source: wgpu::ShaderSource::Wgsl(wgsl_with(casting, 1, Some(&custom)).into()),
        });
        let layouts = [
            Some(&gpu.shadow_layout),
            Some(&layout),
            Some(&gpu.palette_layout),
            Some(&gpu.morph_layout),
        ];
        let count = match (variant.skinned(), variant.morphing()) {
            (_, true) => 4,
            (true, false) => 3,
            (false, false) => 2,
        };
        shadow_pipeline(device, &module, &layouts[..count], casting)
    });
    match made {
        Ok(pipeline) => {
            gpu.custom.shadows.insert((shader.id, variant), pipeline);
        }
        Err(error) => refused(gpu, shader, (variant, true), error),
    }
}
/// What the device refused of `shader`'s pipeline for a variant (its
/// shadows' if `.1`): kept, so its meshes draw without it and it isn't
/// made again each frame, and said by `World::shader_error`.
#[cold]
#[inline(never)]
fn refused(gpu: &mut Gpu, shader: &Shader, which: (Variant, bool), error: String) {
    gpu.custom.failed.insert((shader.id, which.0, which.1));
    shader.failed.replace(Some(error));
}
/// What `make` makes on `device`, or what the device refused of it (a
/// validation or internal error), caught rather than left to the device's
/// handler: an app's shader or texture never stops a frame.
pub(super) fn scoped<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let made = make();
    let errors = [now(internal.pop()), now(validation.pop())];
    match errors.into_iter().flatten().next() {
        None => Ok(made),
        Some(error) => Err(error.to_string()),
    }
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
/// The frame's tail: view, projection, size, time, its step and count, and
/// the surface layers' numbers.
fn write_tail(gpu: &mut Gpu, world: &Inner, frame: &ViewportFrame<'_>) {
    let state = &mut gpu.custom;
    let epoch = *state.epoch.get_or_insert(frame.time);
    let delta = state.last.map_or(0., |last| {
        frame.time.saturating_duration_since(last).as_secs_f32()
    });
    state.last = Some(frame.time);
    let (width, height) = frame.size;
    let aspect = width as f32 / height.max(1) as f32;
    let mut tail: Vec<f32> = Vec::with_capacity(FRAME_TAIL as usize / 4);
    tail.extend(world.camera.view().columns.iter().flatten());
    tail.extend(world.camera.projection(aspect).columns.iter().flatten());
    tail.extend([
        width as f32,
        height as f32,
        frame.time.saturating_duration_since(epoch).as_secs_f32(),
        delta,
        f32::from_bits(state.index),
        0.,
        0.,
        0.,
    ]);
    state.index = state.index.wrapping_add(1);
    let layers = &world.shaders.layers;
    if !layers.is_empty() {
        let (offsets, size) = layer_offsets(layers.iter().map(|l| &**l));
        let start = tail.len();
        tail.resize(start + size as usize / 4, 0.);
        for (layer, at) in layers.iter().zip(offsets) {
            let values = layer.values.borrow();
            let words: Vec<f32> = bytemuck::pod_collect_to_vec(&values.bytes);
            let at = start + at as usize / 4;
            tail[at..at + words.len()].copy_from_slice(&words);
        }
    }
    frame.queue.write_buffer(
        &gpu.uniforms,
        std::mem::size_of::<FrameUniforms>() as u64,
        bytemuck::cast_slice(&tail),
    );
    gpu.written += (tail.len() * 4) as u64;
}

/// The engine's shader as a custom material is checked against: a mesh's
/// (textured, multisampled; cut out by its `opacity` if `opacity`) or a
/// surface layer's (plain).
pub(crate) fn engine_text(mesh: bool, opacity: bool, animated: bool) -> String {
    let variant = if mesh {
        Variant(Variant::TEXTURED | Variant::CUSTOM)
    } else {
        Variant::default()
    };
    wgsl_with(variant, 4, Some(&Custom::bare(opacity, animated)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgx_gpu::wgpu::naga;

    #[test]
    fn the_frames_tail_starts_where_the_engines_uniforms_end() {
        // `write_tail` writes at `FrameUniforms`' size: the shader's `view`
        // must be there, and `layers` where `layer_offsets` counts from.
        let custom = Custom::new(None, std::iter::empty());
        let text = wgsl_with(Variant(Variant::TEXTURED), 4, Some(&custom));
        let module = naga::front::wgsl::parse_str(&text).unwrap();
        let members = module
            .types
            .iter()
            .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                (Some(name), naga::TypeInner::Struct { members, .. }) if name == "Frame" => {
                    Some(members.clone())
                }
                _ => None,
            })
            .expect("the Frame struct");
        let offset = |name: &str| {
            members
                .iter()
                .find(|m| m.name.as_deref() == Some(name))
                .map(|m| m.offset)
                .unwrap()
        };
        let base = std::mem::size_of::<FrameUniforms>() as u32;
        assert_eq!(offset("view"), base);
        assert_eq!(offset("layers"), base + 160);
        assert_eq!(FRAME_TAIL, 160 + u64::from(crate::material::LAYER_BYTES));
    }
}
