//! Point and spot lights past the uniform path's eight: binned into clusters
//! (`clusters`) and shaded each by the fragments its clusters hold
//! (design-3d-aaa.md, 6.3). A world shades this way with more point lights
//! than `POINT_LIGHTS`, any with a radius (`PointLight::radius`), or when
//! asked (`World::set_clustered_lights`); any
//! other draws as it did, its pipelines and frame binding untouched.
//!
//! The lists go up in one storage buffer, read as `vec4<u32>`s (the
//! shader's `clusters`): a header (the grid's scales and slicing, the
//! camera's facing, where each part starts), the lights three words each,
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

/// What the lights' lists on the device were binned for.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    view: Mat4,
    projection: Mat4,
    size: (u32, u32),
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
    /// The frame's layouts with the lists: without an environment and with.
    layouts: Option<[wgpu::BindGroupLayout; 2]>,
    /// Their bindings, and the renderer's frame bindings they were made
    /// beside (a shadow map or environment made again makes them again).
    binds: [Option<wgpu::BindGroup>; 2],
    made_from: [Option<wgpu::BindGroup>; 2],
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
    world.for_each_light(|light, _| points += usize::from(matches!(light, Light::Point { .. })));
    points > POINT_LIGHTS
}

/// The frame's lights, before its meshes are prepared: whether it shades
/// through clusters, and if so its lists binned (where a light, the camera
/// or the size changed) and uploaded, and the uniform path's lights turned
/// off. Turning clusters on or off draws the meshes again (their pipelines
/// differ). Once a frame, out of its loop: inlined, the frame's code
/// compiled worse.
#[inline(never)]
pub(super) fn prepare(
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
    // The words, as the shader reads them.
    let binned = &lights.binned;
    let grid = binned.grid.unwrap_or(grid);
    let [slice_scale, slice_bias] = grid.slicing();
    let facing = (camera.target - camera.position).normalized();
    let f = f32::to_bits;
    let words = &mut lights.words;
    words.clear();
    let base_lights = 3;
    let base_clusters = base_lights + 3 * binned.lights.len() as u32;
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
        binned.lights.len() as u32,
    ]);
    words.extend(binned.lights.iter().flat_map(|l| l.map(|v| v.map(f))));
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
        lights.binds = [None, None];
    }
    queue.write_buffer(&lights.buffer.as_ref().unwrap().0, 0, bytes);
    gpu.written += bytes.len() as u64;
    true
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

/// The clustered frame's bindings, made again where the renderer's frame
/// bindings (its shadow map, its environment) or the lists' buffer were.
/// After the frame's shadow map and environment are made.
#[inline(never)]
pub(super) fn bind(gpu: &mut Gpu, device: &wgpu::Device, lit: bool) {
    if !gpu.lights.on {
        return;
    }
    let from = if lit {
        gpu.environment_bind.as_ref()
    } else {
        Some(&gpu.bind)
    };
    let at = usize::from(lit);
    if gpu.lights.binds[at].is_some() && gpu.lights.made_from[at].as_ref() == from {
        return;
    }
    let from = from.cloned();
    make_bind(gpu, device, lit);
    gpu.lights.made_from[at] = from;
}

#[cold]
#[inline(never)]
fn make_bind(gpu: &mut Gpu, device: &wgpu::Device, lit: bool) {
    make_layouts(gpu, device);
    let lights = &gpu.lights;
    let (Some(layouts), Some((buffer, _))) = (&lights.layouts, &lights.buffer) else {
        return;
    };
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
        wgpu::BindGroupEntry {
            binding: 5,
            resource: buffer.as_entire_binding(),
        },
    ];
    if lit {
        let (Some((_, cube)), Some((_, sampler))) = (&gpu.environment, &gpu.environment_layout)
        else {
            return;
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
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3D frame, clustered lights"),
        layout: &layouts[usize::from(lit)],
        entries: &entries,
    });
    gpu.lights.binds[usize::from(lit)] = Some(bind);
}

impl Lights {
    /// The clustered frame's binding, if this frame shades through
    /// clusters.
    pub(super) fn frame_bind(&self, lit: bool) -> Option<&wgpu::BindGroup> {
        self.on.then(|| self.binds[usize::from(lit)].as_ref())?
    }
}

/// The frame's layout for a clustered pipeline, lit by an environment or
/// not (`make_layouts` made it).
pub(super) fn layout_of(gpu: &Gpu, lit: bool) -> &wgpu::BindGroupLayout {
    &gpu.lights.layouts.as_ref().expect("made with the pipeline")[usize::from(lit)]
}

/// The clustered frame's layouts, made once.
#[cold]
#[inline(never)]
pub(super) fn make_layouts(gpu: &mut Gpu, device: &wgpu::Device) {
    if gpu.lights.layouts.is_some() {
        return;
    }
    let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    };
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let plain = [
        entry(
            0,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
        entry(
            1,
            fragment,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ),
        entry(
            2,
            fragment,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        ),
        entry(
            5,
            fragment,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
    ];
    let mut lit = plain.to_vec();
    lit.push(entry(
        3,
        fragment,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::Cube,
            multisampled: false,
        },
    ));
    lit.push(entry(
        4,
        fragment,
        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
    ));
    let layout = |entries: &[wgpu::BindGroupLayoutEntry]| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D frame, clustered lights"),
            entries,
        })
    };
    gpu.lights.layouts = Some([layout(&plain), layout(&lit)]);
    // The environment's sampler, which a lit binding takes.
    if gpu.environment_layout.is_none() {
        gpu.environment_layout = Some(super::frame::environment_layout(device));
    }
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
        assert!(!gpu.lights.on && gpu.lights.layouts.is_none() && gpu.lights.buffer.is_none());
        world.set_clustered_lights(true);
        draw(&mut renderer, &device, &queue);
        assert!(renderer.gpu.as_ref().unwrap().lights.on);
    }
}
