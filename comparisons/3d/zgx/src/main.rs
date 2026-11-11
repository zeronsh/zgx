//! The zgx-3d arm: the scene drawn by a world's painter into an offscreen
//! target on a device of our own (so timestamp queries can be asked for),
//! the camera moving along the path every frame, no window and no UI
//! compositing (the other engines have none).
//!
//! GPU time is read from two timestamps a frame, written by a one-pixel
//! pass recorded just before the world's passes and one just after, in the
//! same encoder. CPU time is the process's (every thread), and the painter's
//! recording plus the submit alone. Then, at rest, the same world in a zgx
//! window's frame loop (headless renderer): frames that ran a pass while
//! nothing changed.
#[path = "../../common/report.rs"]
mod report;
#[path = "../../common/rusage.rs"]
mod rusage;

use std::{collections::VecDeque, path::Path, time::Instant};
use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::{
    GpuRenderer,
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// The system allocator, counting the bytes live on the heap, so the
/// footprint splits into heap and everything else (GPU, driver, code).
struct Counting;
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        LIVE.fetch_sub(layout.size(), std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        LIVE.fetch_add(size, std::sync::atomic::Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
fn heap() -> u64 {
    LIVE.load(std::sync::atomic::Ordering::Relaxed) as u64
}
/// The background every arm clears to, sRGB.
const CLEAR: u32 = 0x6b7a8f;
/// Frames recorded ahead of the GPU at most, as a swap chain allows.
const IN_FLIGHT: usize = 2;

fn main() {
    let setup = report::setup();
    let mut errors = Vec::new();
    // The footprint at each stage, to split it by what holds it, and (once
    // there's a device) what the Metal device says it has allocated.
    let mut stages: Vec<(&str, u64, Option<u64>, u64)> =
        vec![("start", rusage::sample().footprint, None, heap())];

    // Load: from the files' bytes to the first frame drawn.
    let load_started = Instant::now();
    let mut load_ms = 0.;
    let mut load_peak = 0;
    let started = Instant::now();
    let environment = Environment::decode_hdr(&std::fs::read(&setup.environment).unwrap())
        .expect("the environment");
    let environment_ms = started.elapsed().as_secs_f64() * 1e3;
    stages.push(("environment made", rusage::sample().footprint, None, heap()));
    let started = Instant::now();
    // `boxes:N:M` instead of a file: the generated instance scene (see
    // run.py), N cubes of one geometry and material, the first M moving.
    let boxes = report::boxes(&setup.model);
    // Load phases: the files' reads (timed inside the resolver), and the
    // images read, to decode them again apart afterwards.
    // `--load-phases=1`: also decode the images again apart, to time that
    // phase (kept out of the load time and off by default: it holds the
    // files' bytes meanwhile).
    let phases = std::env::args().any(|a| a == "--load-phases=1");
    let mut read_ms = 0.;
    let mut images: Vec<Vec<u8>> = Vec::new();
    let generated = boxes.is_some()
        || report::materials(&setup.model).is_some()
        || report::hierarchy(&setup.model).is_some();
    let model = (!generated).then(|| {
        let path = Path::new(&setup.model);
        let folder = path.parent().unwrap().to_path_buf();
        let read = Instant::now();
        let gltf = std::fs::read(path).unwrap();
        read_ms += read.elapsed().as_secs_f64() * 1e3;
        Model::from_bytes_with(&gltf, |uri| {
            let read = Instant::now();
            let bytes = std::fs::read(folder.join(uri)).ok();
            read_ms += read.elapsed().as_secs_f64() * 1e3;
            if let Some(bytes) = &bytes
                && phases
                && (uri.ends_with(".png") || uri.ends_with(".jpg") || uri.ends_with(".jpeg"))
            {
                images.push(bytes.clone());
            }
            bytes
        })
        .expect("the model")
    });
    let model_ms = started.elapsed().as_secs_f64() * 1e3;
    // Their decoding alone, again, as `Model` decoded them (one thread).
    let decode_started = Instant::now();
    for bytes in &images {
        let _ = Texture::decode(bytes);
    }
    let decode_ms = decode_started.elapsed().as_secs_f64() * 1e3;
    drop(images);
    let mut add_ms = 0.;
    let device_ms: f64;
    let mut first_record_ms = 0.;
    let mut first_wait_ms = 0.;
    let mut first_shown_ms = 0.;
    let mut loading_frames = 0;
    stages.push(("model decoded", rusage::sample().footprint, None, heap()));

    let world = World::new();
    world.set_background(Some(CLEAR));
    // `--shadows=0`: no sun shadows (the A/B for what the shadow pass costs).
    world.set_shadows(!std::env::args().any(|a| a == "--shadows=0"));
    world.set_tone_mapping(ToneMapping::Aces);
    world.set_samples(setup.msaa);
    // `--culling=0`, on a tree that culls: every mesh drawn, to compare.
    #[cfg(feature = "cull-tree")]
    world.set_culling(!std::env::args().any(|a| a == "--culling=0"));
    world.set_environment(Some(environment));
    let sun = |nudge: f32| Light::Directional {
        direction: vec3(setup.sun[0] + nudge, setup.sun[1], setup.sun[2]),
        color: [1., 0.96, 0.9],
        intensity: 3.,
    };
    let sun_id = world.add_light(sun(0.));
    // `--point-lights=N`: the light scene, N shadowless point lights placed
    // as Bevy's and Filament's (`report::light_at`): 150 lm each, so
    // 150 / 4π cd, reaching 8 m. Clustered with the `lighting` feature;
    // without it zgx shades the eight nearest its camera's target.
    let point_lights: usize = std::env::args()
        .find_map(|a| a.strip_prefix("--point-lights=").map(|v| v.parse().unwrap()))
        .unwrap_or(0);
    // `--light-range=R`: their reach (8 m as the others'), to attribute
    // the cost to the lights a pixel shades.
    let range: f32 = std::env::args()
        .find_map(|a| a.strip_prefix("--light-range=").map(|v| v.parse().unwrap()))
        .unwrap_or(8.);
    for i in 0..point_lights {
        let (position, color) = report::light_at(i);
        world.add_light(Light::Point {
            position: position.into(),
            color,
            intensity: 150. / (4. * std::f32::consts::PI),
            range,
        });
    }
    // `--clustered=1`: shaded through clusters whatever the lights (with
    // none, what the clustered shader costs by itself).
    #[cfg(feature = "lighting")]
    world.set_clustered_lights(std::env::args().any(|a| a == "--clustered=1"));
    // zgx redraws its shadow map only when the sun or a caster moves; the
    // other engines redraw theirs every frame. `--shadow-every-frame=1`
    // nudges the sun each frame so zgx does the same work.
    let shadow_every_frame = std::env::args().any(|a| a == "--shadow-every-frame=1");
    // `--post=bloom,lut` (with the `post` feature): the chain the other
    // engines' post columns run: bloom at its default mix, a grade through
    // `report::warm_lut`'s 32³ table; ACES as everywhere. Built with the
    // feature and no `--post`: the chain compiled in and off.
    #[cfg(feature = "post")]
    if let Some(parts) = std::env::args().find_map(|a| a.strip_prefix("--post=").map(str::to_owned)) {
        let on = |part: &str| parts.split(',').any(|p| p == part);
        world.set_post(Post {
            bloom: on("bloom").then(Bloom::default),
            grade: on("lut").then(|| Grade {
                lut: Some(Lut::from_strip(32 * 32, 32, &report::warm_lut(32)).expect("a strip")),
                ..Grade::default()
            }),
            ..Post::default()
        });
    }
    if let Some(model) = &model {
        let added = Instant::now();
        world.add_model(model);
        add_ms = added.elapsed().as_secs_f64() * 1e3;
    }
    let cubes: Vec<ObjectId> = match boxes {
        Some((count, _)) => {
            let cube = Geometry::cuboid(1., 1., 1.);
            let material = Material::rgb(0xc8c0b4).with(0., 0.6);
            (0..count)
                .map(|i| {
                    let [x, y, z] = report::box_at(i, count, 0);
                    world.add(Object::mesh(cube.clone(), material).at([x, y, z]))
                })
                .collect()
        }
        None => Vec::new(),
    };
    // `materials:N` and `hierarchy:D:F` (see report.rs): shapes the instance
    // and culling work wasn't tuned on.
    let mut cubes = cubes;
    let mut spinners: Vec<ObjectId> = Vec::new();
    if let Some(count) = report::materials(&setup.model) {
        let shapes: Vec<Geometry> = (0..8)
            .map(|i| match i % 4 {
                0 => Geometry::cuboid(1., 1. + i as f32 * 0.1, 1.),
                1 => Geometry::sphere(0.6, 12 + i),
                2 => Geometry::cylinder(0.5, 1., 10 + i),
                _ => Geometry::torus(0.5, 0.15, 12 + i),
            })
            .collect();
        let textures: Vec<Texture> = (0..64u32)
            .map(|t| {
                let v = (t * 4) as u8;
                Texture::from_rgba8(4, 4, [v, 255 - v, 128, 255].repeat(16)).unwrap()
            })
            .collect();
        for i in 0..count {
            let [x, y, z] = report::box_at(i, count, 0);
            let colour = (i as u32).wrapping_mul(2654435761) >> 8;
            let material =
                Material::rgb(colour).with((i % 7) as f32 / 6., 0.2 + (i % 5) as f32 * 0.2);
            let id =
                world.add(Object::mesh(shapes[i % shapes.len()].clone(), material).at([x, y, z]));
            if i % 4 == 0 {
                world.set_texture(id, Some(textures[i % textures.len()].clone()));
            }
            cubes.push(id);
        }
    }
    if let Some((depth, fanout)) = report::hierarchy(&setup.model) {
        let cube = Geometry::cuboid(0.8, 0.8, 0.8);
        let material = Material::rgb(0xc8c0b4).with(0., 0.6);
        // Each level's children spread on a ring a quarter the size of the
        // last's; the leaves are cubes.
        fn grow(
            world: &World,
            parent: ObjectId,
            level: usize,
            depth: usize,
            fanout: usize,
            radius: f32,
            cube: &Geometry,
            material: Material,
            leaves: &mut Vec<ObjectId>,
        ) {
            for c in 0..fanout {
                let a = c as f32 / fanout as f32 * std::f32::consts::TAU;
                let at = [radius * a.cos(), 0., radius * a.sin()];
                if level + 1 == depth {
                    leaves.push(
                        world.add(Object::mesh(cube.clone(), material).at(at).child_of(parent)),
                    );
                } else {
                    let node = world.add(Object::group().at(at).child_of(parent));
                    grow(
                        world,
                        node,
                        level + 1,
                        depth,
                        fanout,
                        radius * 0.3,
                        cube,
                        material,
                        leaves,
                    );
                }
            }
        }
        let root = world.add(Object::group());
        for c in 0..fanout {
            let a = c as f32 / fanout as f32 * std::f32::consts::TAU;
            let top = world.add(
                Object::group()
                    .at([60. * a.cos(), 1., 60. * a.sin()])
                    .child_of(root),
            );
            spinners.push(top);
            grow(
                &world, top, 1, depth, fanout, 18., &cube, material, &mut cubes,
            );
        }
    }
    let spinner_at: Vec<Vec3> = spinners
        .iter()
        .map(|&id| world.object(id).unwrap().transform.position)
        .collect();
    let moving = boxes.map_or(0, |(_, moving)| moving);
    let camera = |(position, target): ([f32; 3], [f32; 3])| Camera {
        fov: setup.fov.to_radians(),
        near: 0.1,
        far: 300.,
        ..Camera::looking_at(position, target)
    };
    world.set_camera(camera(setup.path[0]));

    // A device of our own, with timestamps where the adapter has them.
    let device_started = Instant::now();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("an adapter");
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: if timestamps {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        },
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("a device");
    device_ms = device_started.elapsed().as_secs_f64() * 1e3;
    let (width, height) = (setup.width, setup.height);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("The frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[FORMAT.add_srgb_suffix()],
    });
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    let frames = setup.warmup + setup.path.len();
    let queries = timestamps.then(|| {
        device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: (frames * 2) as u32,
        })
    });
    // Empty passes over the frame's own target, so the GPU can't run them
    // beside the world's (a pass on a texture of its own ran alongside them,
    // and read 0.004 ms): the first's start, the last's end.
    let mark = |encoder: &mut wgpu::CommandEncoder, index: u32| {
        let Some(queries) = &queries else { return };
        let first = index % 2 == 0;
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Timestamp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: queries,
                beginning_of_pass_write_index: first.then_some(index),
                end_of_pass_write_index: (!first).then_some(index),
            }),
            ..Default::default()
        });
    };

    stages.push((
        "device made",
        rusage::sample().footprint,
        allocated(&device),
        heap(),
    ));
    let mut painter = world.viewport();
    // Meshes each measured frame drew (a tree with `World::culled`).
    let meshes = world_meshes(&cubes);
    #[cfg_attr(not(feature = "cull-tree"), allow(unused_mut))]
    let mut drawn_meshes: Vec<f64> = Vec::new();
    // `--drop-model=1`: the model let go once added (the world holds what
    // it draws), to see what holding it costs.
    let mut model = model;
    if std::env::args().any(|a| a == "--drop-model=1") {
        model = None;
    }
    let mut submit_ms = Vec::new();
    let mut frame_ms = Vec::new();
    let mut in_flight = VecDeque::new();
    let mut before = rusage::sample();
    let mut counted_from = Instant::now();
    let mut last = Instant::now();
    for frame in 0..frames {
        if frame == 1 {
            let waited = Instant::now();
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the device");
            first_wait_ms = waited.elapsed().as_secs_f64() * 1e3;
            load_ms = load_started.elapsed().as_secs_f64() * 1e3 - decode_ms;
            load_peak = rusage::sample().peak_footprint;
            stages.push((
                "first frame drawn",
                rusage::sample().footprint,
                allocated(&device),
                heap(),
            ));
            if std::env::args().any(|a| a == "--relieve=1") {
                unsafe extern "C" {
                    fn malloc_zone_pressure_relief(
                        zone: *mut std::ffi::c_void,
                        goal: usize,
                    ) -> usize;
                }
                unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
                stages.push((
                    "first frame, malloc relieved",
                    rusage::sample().footprint,
                    allocated(&device),
                    heap(),
                ));
            }
        }
        if frame == setup.warmup {
            stages.push((
                "warmed up",
                rusage::sample().footprint,
                allocated(&device),
                heap(),
            ));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the device");
            gpu_spans();
            gpu_intervals();
            in_flight.clear();
            before = rusage::sample();
            counted_from = Instant::now();
            last = counted_from;
        }
        let pose = setup.path[frame.saturating_sub(setup.warmup)];
        let start = Instant::now();
        world.set_camera(camera(pose));
        for (i, &cube) in cubes.iter().take(moving).enumerate() {
            world.set_transform(cube, Transform::at(report::box_at(i, cubes.len(), frame)));
        }
        for (i, (&top, &at)) in spinners.iter().zip(&spinner_at).enumerate() {
            world.set_transform(
                top,
                Transform {
                    position: at,
                    rotation: Quat::from_axis_angle(Vec3::Y, frame as f32 * 0.02 + i as f32),
                    ..Transform::default()
                },
            );
        }
        if shadow_every_frame {
            world.set_light(sun_id, sun(if frame % 2 == 0 { 1e-4 } else { 0. }));
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        mark(&mut encoder, (frame * 2) as u32);
        {
            let mut drawing = ViewportFrame::new(
                &device,
                &queue,
                &mut encoder,
                &view,
                FORMAT,
                (width, height),
                1.,
            );
            drawing.target_srgb = Some(&srgb);
            let recording = Instant::now();
            painter.render(&mut drawing);
            if frame == 0 {
                first_record_ms = recording.elapsed().as_secs_f64() * 1e3;
            }
        }
        mark(&mut encoder, (frame * 2 + 1) as u32);
        in_flight.push_back(queue.submit([encoder.finish()]));
        // Maps still on their way: drawn as a display would, a frame each
        // 120 Hz refresh, until the world has them all. The load ends at the
        // first frame drawn with every map.
        if frame == 0 {
            first_shown_ms = load_started.elapsed().as_secs_f64() * 1e3 - decode_ms;
            while world.loading() {
                std::thread::sleep(std::time::Duration::from_micros(8_333));
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut drawing = ViewportFrame::new(
                        &device,
                        &queue,
                        &mut encoder,
                        &view,
                        FORMAT,
                        (width, height),
                        1.,
                    );
                    drawing.target_srgb = Some(&srgb);
                    painter.render(&mut drawing);
                }
                in_flight.push_back(queue.submit([encoder.finish()]));
                loading_frames += 1;
                while in_flight.len() > IN_FLIGHT {
                    let index = in_flight.pop_front().unwrap();
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(index),
                            timeout: None,
                        })
                        .expect("the device");
                }
            }
        }
        let submitted = Instant::now();
        if frame >= setup.warmup {
            submit_ms.push((submitted - start).as_secs_f64() * 1e3);
            #[cfg(feature = "cull-tree")]
            drawn_meshes.push(meshes.saturating_sub(world.culled() as usize) as f64);
        }
        // As a swap chain would: wait only when too far ahead.
        while in_flight.len() > IN_FLIGHT {
            let index = in_flight.pop_front().unwrap();
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .expect("the device");
        }
        if frame >= setup.warmup {
            let now = Instant::now();
            frame_ms.push((now - last).as_secs_f64() * 1e3);
            last = now;
        }
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device");
    let elapsed = counted_from.elapsed().as_secs_f64();
    let after = rusage::sample();
    // Each frame's submission as Metal timed it (the patched wgpu-hal).
    let spans_ms: Vec<f64> = gpu_spans().into_iter().map(|s| s * 1e3).collect();
    let (busy, covered) = report::busy(gpu_intervals());
    let counted = setup.path.len() as f64;

    // The GPU's times, frame by frame.
    let gpu_ms: Vec<f64> = match &queries {
        Some(queries) => {
            let bytes = (frames * 2 * 8) as u64;
            let resolved = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Resolved timestamps"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Timestamps read"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.resolve_query_set(queries, 0..(frames * 2) as u32, &resolved, 0);
            encoder.copy_buffer_to_buffer(&resolved, 0, &read, 0, bytes);
            queue.submit([encoder.finish()]);
            let ticks: Vec<u64> = map(&device, &read, |data| {
                bytemuck::cast_slice::<u8, u64>(data).to_vec()
            });
            let period = queue.get_timestamp_period() as f64;
            (setup.warmup..frames)
                .map(|f| ticks[f * 2 + 1].saturating_sub(ticks[f * 2]) as f64 * period / 1e6)
                .collect()
        }
        None => {
            errors.push("no timestamp queries on this adapter".to_owned());
            Vec::new()
        }
    };

    // The last frame's pixels.
    let row = (width * 4).div_ceil(256) * 256;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame read"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let rgba: Vec<u8> = map(&device, &read, |data| {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let at = (y * row) as usize;
            for px in data[at..at + (width * 4) as usize].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // BGRA
            }
        }
        rgba
    });
    report::write_png(&setup.out, width, height, &rgba);
    let clear = [(CLEAR >> 16) as u8, (CLEAR >> 8) as u8, CLEAR as u8];
    let (hash, drawn) = report::proof(&rgba, clear);
    let footprint_drawing = after.footprint;
    stages.push(("drawing", footprint_drawing, allocated(&device), heap()));
    // At rest: zgx draws nothing while nothing changes, so its rest is this
    // state, idle. `--rest-s=0` skips it.
    let rest_s: u64 = std::env::args()
        .find_map(|a| a.strip_prefix("--rest-s=").map(|v| v.parse().unwrap()))
        .unwrap_or(10);
    std::thread::sleep(std::time::Duration::from_secs(rest_s));
    let footprint_rested = rusage::sample().footprint;
    stages.push(("rested", footprint_rested, allocated(&device), heap()));
    // `--pause=S`: hold here S seconds, for `footprint` from outside.
    if let Some(s) = std::env::args().find_map(|a| a.strip_prefix("--pause=").map(str::to_owned)) {
        eprintln!("paused, pid {}", std::process::id());
        std::thread::sleep(std::time::Duration::from_secs(s.parse().unwrap()));
    }
    drop(model.take());
    stages.push((
        "model dropped",
        rusage::sample().footprint,
        allocated(&device),
        heap(),
    ));
    drop(painter);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device");
    stages.push((
        "painter dropped",
        rusage::sample().footprint,
        allocated(&device),
        heap(),
    ));
    // What the allocator kept of what was freed: given back, and counted again.
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        eprintln!("malloc released {:.1} MB", released as f64 / 1048576.);
        stages.push((
            "malloc relieved",
            rusage::sample().footprint,
            allocated(&device),
            heap(),
        ));
    }
    let stages = stages
        .iter()
        .map(|(stage, bytes, gpu, heap)| {
            let mb = |b: u64| b as f64 / 1048576.;
            match gpu {
                Some(gpu) => format!(
                    "{stage}: {:.1} (heap {:.1}, Metal {:.1})",
                    mb(*bytes),
                    mb(*heap),
                    mb(*gpu)
                ),
                None => format!("{stage}: {:.1} (heap {:.1})", mb(*bytes), mb(*heap)),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let rest = at_rest(
        &world,
        setup.path[setup.path.len() - 1],
        &camera,
        &mut errors,
    );

    let mut json = report::Json::default();
    json.text(
        "arm",
        if shadow_every_frame {
            "zgx-shadow-every-frame"
        } else {
            "zgx"
        },
    )
    .int("gpu_spans", spans_ms.len() as u64)
    .int("meshes", meshes as u64)
    .spread("drawn_meshes", &drawn_meshes)
    .num("gpu_ms_per_frame", busy * 1e3 / counted)
    .num(
        "gpu_busy_fraction",
        if covered > 0. {
            busy / covered
        } else {
            f64::NAN
        },
    )
    .text("adapter", &adapter.get_info().name)
    .int("frames", counted as u64)
    .num("environment_ms", environment_ms)
    .num("model_ms", model_ms)
    .spread("gpu_ms", &spans_ms)
    .spread("gpu_marker_ms", &gpu_ms)
    .spread("frame_ms", &frame_ms)
    .spread("submit_ms", &submit_ms)
    .num("fps", counted / elapsed)
    .num(
        "cpu_ms_per_frame",
        (after.cpu_ns - before.cpu_ns) / 1e6 / counted,
    )
    .num(
        "instructions_per_frame",
        (after.instructions - before.instructions) as f64 / counted,
    )
    .num(
        "cycles_per_frame",
        (after.cycles - before.cycles) as f64 / counted,
    )
    .num("load_ms", load_ms)
    .text(
        "load_phases_ms",
        &format!(
            "environment (decode + CPU prefilter) {environment_ms:.0}; files read {read_ms:.0}; \
model (parse + decode + build) {model_ms:.0}, of which decode alone ~{decode_ms:.0}; \
add_model {add_ms:.0}; device {device_ms:.0}; first frame recorded (mips, uploads, \
pipelines, instances) {first_record_ms:.0}; first frame on the GPU (wait) {first_wait_ms:.0}; \
first frame shown at {first_shown_ms:.0}, then {loading_frames} frames till every map was in"
        ),
    )
    .num("load_peak_footprint_mb", load_peak as f64 / 1048576.)
    .num("footprint_mb", footprint_drawing as f64 / 1048576.)
    .num("rested_footprint_mb", footprint_rested as f64 / 1048576.)
    .int("rest_seconds", rest_s)
    .num("peak_footprint_mb", after.peak_footprint as f64 / 1048576.)
    .int("hash", hash)
    .num("drawn_fraction", drawn)
    .int("rest_ticks", rest.ticks)
    .int("rest_frames", rest.frames)
    .int("rest_control_frames", rest.control)
    .num("rest_cpu_ms_per_tick", rest.cpu_ms / rest.ticks as f64)
    .text("footprint_stages_mb", &stages)
    .text("errors", &errors.join("; "));
    println!("{}", json.line());
}

struct Rest {
    ticks: u64,
    /// Ticks at rest that ran a GPU pass.
    frames: u64,
    /// Ticks after a change that did (must be 1: the arm can see a draw).
    control: u64,
    cpu_ms: f64,
}

/// The world in a zgx window's frame loop (a headless renderer, as zgx's own
/// tests draw), ticked as a 120 Hz display would for ten seconds while
/// nothing changes. A control tick after a camera move proves a frame shows.
fn at_rest(
    world: &World,
    pose: ([f32; 3], [f32; 3]),
    camera: &dyn Fn(([f32; 3], [f32; 3])) -> Camera,
    errors: &mut Vec<String>,
) -> Rest {
    let (w, h) = (640., 360.);
    let mut gpu = match GpuRenderer::new(w as u32, h as u32) {
        Ok(gpu) => gpu,
        Err(error) => {
            errors.push(format!("no renderer for the rest phase: {error}"));
            return Rest {
                ticks: 0,
                frames: 0,
                control: 0,
                cpu_ms: 0.,
            };
        }
    };
    let mut ui = Ui::new(w, h);
    ui.mount(
        div()
            .size(w, h)
            .bg(Color(0, 0, 0, 255))
            .child(gpu_view("World", world.view()).absolute().size(w, h)),
    );
    let mut tick = |ui: &mut Ui| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage)
            .unwrap()
            .render_passes
    };
    world.set_camera(camera(pose));
    for _ in 0..8 {
        tick(&mut ui); // its first frames: pipelines, textures, the view's
    }
    // Ten seconds, as the other arms' (`--rest-ticks=N` to change).
    let ticks: u64 = std::env::args()
        .find_map(|a| a.strip_prefix("--rest-ticks=").map(|v| v.parse().unwrap()))
        .unwrap_or(1200);
    let before = rusage::sample();
    let mut frames = 0;
    for _ in 0..ticks {
        let started = Instant::now();
        if tick(&mut ui) > 0 {
            frames += 1;
        }
        std::thread::sleep(
            std::time::Duration::from_micros(8333).saturating_sub(started.elapsed()),
        );
    }
    let after = rusage::sample();
    let mut moved = camera(pose);
    moved.position.y += 0.1;
    world.set_camera(moved);
    let control = (tick(&mut ui) > 0) as u64;
    Rest {
        ticks,
        frames,
        control,
        cpu_ms: (after.cpu_ns - before.cpu_ns) / 1e6,
    }
}

/// What the Metal device says it has allocated: textures, buffers, and
/// the driver's own (none elsewhere).
fn allocated(device: &wgpu::Device) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        use objc2_metal::MTLDevice;
        // SAFETY: only queried.
        let metal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
        Some(metal.raw_device().currentAllocatedSize() as u64)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = device;
        None
    }
}

/// Meshes in the world: the generated cubes, or the model's primitives
/// (counted in the file the arm read: the API lists no meshes).
fn world_meshes(cubes: &[ObjectId]) -> usize {
    if !cubes.is_empty() {
        return cubes.len();
    }
    std::env::args()
        .find_map(|a| a.strip_prefix("--model=").map(str::to_owned))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or(0, |json| json.matches("\"attributes\"").count())
}

/// Command buffers' GPU intervals since last asked (Metal only).
fn gpu_intervals() -> Vec<(f64, f64)> {
    #[cfg(all(target_os = "macos", feature = "gpu-time"))]
    return zgx_gpu::wgpu::hal::metal::gpu_time::take_intervals();
    #[cfg(not(all(target_os = "macos", feature = "gpu-time")))]
    Vec::new()
}

/// Submissions' GPU spans, in seconds, since last asked (Metal only).
fn gpu_spans() -> Vec<f64> {
    #[cfg(all(target_os = "macos", feature = "gpu-time"))]
    return zgx_gpu::wgpu::hal::metal::gpu_time::take_spans();
    #[cfg(not(all(target_os = "macos", feature = "gpu-time")))]
    Vec::new()
}

fn map<T>(device: &wgpu::Device, buffer: &wgpu::Buffer, f: impl FnOnce(&[u8]) -> T) -> T {
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("mapped"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device");
    let value = f(&slice.get_mapped_range());
    buffer.unmap();
    value
}
