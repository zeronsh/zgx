//! Particle emitters: they spawn, move, collide and age alike on the GPU and
//! the CPU, show where they are, and keep the world drawing only while any
//! may live. Drawn on a real device (`ZGX_REQUIRE_GPU`; Metal here) at frame
//! times of the test's own, so runs repeat.
#![cfg(feature = "particles")]
use std::time::{Duration, Instant};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

const SIDE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

fn device(limits: wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let found = pollster(instance.request_adapter(&Default::default()))
        .ok()
        .and_then(|adapter| {
            pollster(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))
            .ok()
        });
    if found.is_none() && std::env::var_os("ZGX_REQUIRE_GPU").is_some() {
        panic!("ZGX_REQUIRE_GPU, but no device");
    }
    found
}

fn pollster<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A world's painter on a device of the test's own, drawing at times it
/// chooses.
struct Frames {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: wgpu::Texture,
    painter: Box<dyn Viewport>,
    start: Instant,
    /// Times its painter draws each frame (a view shown in that many boxes).
    boxes: usize,
    /// A second painter of the world (`World::viewport`) and its target,
    /// drawing each frame after the first (or before: `second_first`).
    second: Option<(Box<dyn Viewport>, wgpu::Texture)>,
    second_first: bool,
}
impl Frames {
    fn new(world: &World) -> Option<Self> {
        Self::with_limits(world, wgpu::Limits::default())
    }
    /// On a device asked for `limits`.
    fn with_limits(world: &World, limits: wgpu::Limits) -> Option<Self> {
        let (device, queue) = device(limits)?;
        Some(Self {
            target: target(&device),
            device,
            queue,
            painter: Box::new(world.viewport()),
            start: Instant::now(),
            boxes: 1,
            second: None,
            second_first: false,
        })
    }
    /// With a second painter of `world` beside the first, on its device.
    fn with_second(mut self, world: &World) -> Self {
        self.second = Some((Box::new(world.viewport()), target(&self.device)));
        self
    }
    /// A frame at `seconds` of frame time.
    fn draw(&mut self, seconds: f64) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let time = self.start + Duration::from_secs_f64(seconds);
        if self.second_first
            && let Some((painter, target)) = &mut self.second
        {
            let gpu = (&self.device, &self.queue, &mut encoder);
            paint(&mut **painter, gpu, target, time);
        }
        for _ in 0..self.boxes {
            paint(
                &mut *self.painter,
                (&self.device, &self.queue, &mut encoder),
                &self.target,
                time,
            );
        }
        if !self.second_first
            && let Some((painter, target)) = &mut self.second
        {
            paint(
                &mut **painter,
                (&self.device, &self.queue, &mut encoder),
                target,
                time,
            );
        }
        self.queue.submit([encoder.finish()]);
    }
    /// The second painter's target's pixels, BGRA.
    fn second_pixels(&self) -> Vec<u8> {
        pixels_of(&self.device, &self.queue, &self.second.as_ref().unwrap().1)
    }
    /// The target's pixels, BGRA.
    fn pixels(&self) -> Vec<u8> {
        pixels_of(&self.device, &self.queue, &self.target)
    }
}

impl Frames {
    /// Frames at `from` until the world's particles' pipelines are made
    /// (on their thread): its particles step from the next.
    fn ready(&mut self, world: &World, from: f64) {
        // (Up to 10 s of wall time: compiling isn't what's tested.)
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            self.draw(from);
            if world.particles_ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("the particles' pipelines weren't made");
    }
    /// Frames at 60 a second after `from` until `until`: the last frame's
    /// time.
    fn run(&mut self, from: f64, until: f64) -> f64 {
        let mut at = from;
        while at + 1. / 60. <= until + 1e-9 {
            at += 1. / 60.;
            self.draw(at);
        }
        at
    }
}

/// A target of the frames' size and format.
fn target(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[FORMAT.add_srgb_suffix()],
    })
}

/// `painter`'s frame at `time` into `target`.
fn paint(
    painter: &mut dyn Viewport,
    (device, queue, encoder): (&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder),
    target: &wgpu::Texture,
    time: Instant,
) {
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    let mut frame = ViewportFrame::new(device, queue, encoder, &view, FORMAT, (SIDE, SIDE), 1.);
    frame.target_srgb = Some(&srgb);
    frame.time = time;
    painter.render(&mut frame);
}

/// `target`'s pixels, BGRA.
fn pixels_of(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    let row = (SIDE * 4).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * SIDE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    (0..SIDE as usize)
        .flat_map(|y| mapped[y * row as usize..][..SIDE as usize * 4].to_vec())
        .collect()
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * SIDE as usize + x) * 4;
    let [b, g, r, a] = pixels[at..at + 4].try_into().unwrap();
    [r, g, b, a]
}

/// A world looking at the origin from 4 back, an opaque black background.
fn world() -> World {
    let world = World::new();
    world.set_background(Some(0x000000));
    world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
    world.set_tone_mapping(ToneMapping::Linear);
    world
}

#[test]
fn a_burst_shows_where_it_was_born_until_its_life_ends() {
    let world = world();
    world
        .add_emitter(
            Emitter::new(16)
                .spawn(Spawn::point([0., 0., 0.]))
                .burst(16)
                .lifetime(0.5..0.5)
                .size(1.0..1.0)
                .color([1., 0.5, 0.25, 1.]),
        )
        .unwrap();
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    // The pipelines made; the burst waits for them.
    frames.ready(&world, 0.);
    frames.draw(1. / 60.);
    let pixels = frames.pixels();
    let [r, g, b, _] = pixel(&pixels, 32, 32);
    // Linear 1, 0.5, 0.25 in sRGB: 255, 188, 137; sixteen discs over one
    // another, opaque at their middles.
    assert!(
        r > 245 && (178..198).contains(&g) && (127..147).contains(&b),
        "{:?}",
        [r, g, b]
    );
    assert_eq!(
        pixel(&pixels, 2, 2),
        [0, 0, 0, 255],
        "outside: the background"
    );
    // Its life over, nothing shows.
    frames.run(1. / 60., 0.6);
    assert_eq!(pixel(&frames.pixels(), 32, 32), [0, 0, 0, 255]);
}

#[test]
fn particles_fall_and_collide_alike_on_the_gpu_and_the_cpu() {
    // The GPU's, the CPU's, and the GPU's again: the same frames, however
    // long its pipelines took.
    let images: Vec<Vec<u8>> = [false, true, false]
        .into_iter()
        .filter_map(|on_cpu| {
            let world = world();
            let mut frames = Frames::new(&world)?;
            world.set_particles_on_cpu(on_cpu);
            let rain = world
                .add_emitter(
                    Emitter::new(2000)
                        .spawn(Spawn::box_([-1., 1., -0.2], [1., 1.2, 0.2]).rate(3000.))
                        .lifetime(2.0..2.0)
                        .velocity([0., 0., 0.], 0.5)
                        .force(Force::Gravity)
                        .force(Force::Drag(0.5))
                        .collide(Collide::Plane {
                            height: -0.5,
                            then: Response::Bounce {
                                keep: 0.3,
                                friction: 0.2,
                            },
                        })
                        .size(0.05..0.08)
                        .color([0.2, 0.6, 1.0, 1.0]),
                )
                .unwrap();
            world.set_emitting(rain, false);
            frames.ready(&world, 0.);
            world.set_emitting(rain, true);
            frames.run(0., 50. / 60.);
            Some(frames.pixels())
        })
        .collect();
    let [gpu, cpu, again] = &images[..] else {
        return;
    };
    assert!(gpu == again, "the same frames draw the same particles");
    // Lit where they are: below the spawn box and above the plane, nothing
    // under it.
    let lit = |image: &[u8], rows: std::ops::Range<usize>| {
        rows.flat_map(|y| (0..SIDE as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(image, x, y)[2] > 40)
            .count()
    };
    // The plane at y = -0.5, 4 away with a 45° view: about row 40.
    let (above, below) = (lit(gpu, 0..39), lit(gpu, 44..64));
    assert!(above > 200, "{above} lit above the plane");
    assert_eq!(below, 0, "nothing under the plane");
    // The CPU's: the same particles, so nearly the same pixels.
    let differ = gpu
        .chunks(4)
        .zip(cpu.chunks(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 24))
        .count();
    assert!(
        differ < 40,
        "{differ} pixels differ between the GPU's and the CPU's"
    );
}

#[test]
fn a_world_draws_while_particles_may_live_and_then_settles() {
    let world = world();
    assert!(!world.view().animating());
    let rain = world
        .add_emitter(
            Emitter::new(1000)
                .spawn(Spawn::box_([-1., 1., -1.], [1., 1., 1.]).rate(500.))
                .lifetime(0.3..0.4)
                .velocity([0., -5., 0.], 0.)
                .draw(Shape::Streak { length: 0.05 }, Blend::Additive),
        )
        .unwrap();
    assert!(world.view().animating(), "emitting");
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    frames.ready(&world, 0.);
    let stopped = frames.run(0., 0.5);
    assert!(world.view().animating());
    world.set_emitting(rain, false);
    // Its particles live out their lives: at most 0.4 s, and a step.
    let mut at = stopped;
    while world.view().animating() {
        at = frames.run(at, at + 1. / 60.);
        assert!(at - stopped < 1., "never settled");
    }
    let lived = at - stopped;
    assert!(
        (0.4..0.55).contains(&lived),
        "settled {lived} s after it stopped"
    );
    // A burst wakes it, for that burst's lives.
    world.burst(rain, 10);
    assert!(world.view().animating());
    frames.run(at, at + 0.6);
    assert!(!world.view().animating());
}

#[test]
fn the_cpu_holds_at_most_its_cap() {
    let world = world();
    world.add_emitter(Emitter::new(100_000)).unwrap();
    let gpu = world.memory().particles;
    assert_eq!(gpu, 100_000 * 80);
    world.set_particles_on_cpu(true);
    assert_eq!(world.memory().particles, u64::from(CPU_PARTICLES) * 80);
}

#[test]
fn drops_hitting_a_plane_give_birth_to_splashes_there() {
    // Red where the drops hit, on the GPU and the CPU; none without the
    // sub-emitter.
    let reds: Vec<usize> = [(false, true), (true, true), (false, false)]
        .into_iter()
        .filter_map(|(on_cpu, splashing)| {
            let world = world();
            let mut frames = Frames::new(&world)?;
            world.set_particles_on_cpu(on_cpu);
            let splashes = world
                .add_emitter(
                    Emitter::new(4000)
                        .spawn(Spawn::sphere([0., 0.02, 0.], 0.02))
                        .lifetime(0.3..0.3)
                        .velocity([0., 1., 0.], 0.5)
                        .size(0.06..0.06)
                        .color([1., 0.05, 0.05, 1.]),
                )
                .unwrap();
            let mut drops = Emitter::new(2000)
                .spawn(Spawn::box_([-1., 1., -0.1], [1., 1.1, 0.1]).rate(600.))
                .lifetime(2.0..2.0)
                .force(Force::Gravity)
                .collide(Collide::Plane {
                    height: -0.5,
                    then: Response::Die,
                })
                .size(0.03..0.03)
                .color([0.2, 0.6, 1.0, 1.0]);
            if splashing {
                drops = drops.on_hit(splashes, 3);
            }
            let rain = world.add_emitter(drops).unwrap();
            world.set_emitting(rain, false);
            frames.ready(&world, 0.);
            world.set_emitting(rain, true);
            let at = frames.run(0., 50. / 60.);
            let image = frames.pixels();
            let red = (30..46)
                .flat_map(|y| (0..SIDE as usize).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let [r, g, b, _] = pixel(&image, x, y);
                    r > 150 && g < 100 && b < 100
                })
                .count();
            // Stopped, it settles once the drops have fallen and the last
            // splashes are gone: 2 s and 0.3 s at most, and a step.
            world.set_emitting(rain, false);
            let mut until = at;
            while world.view().animating() {
                until = frames.run(until, until + 1. / 60.);
                assert!(until - at < 3., "never settled");
            }
            Some(red)
        })
        .collect();
    let [gpu, cpu, none] = reds[..] else {
        return;
    };
    assert!(
        gpu > 100 && cpu > 100,
        "splashes: {gpu} red on the GPU, {cpu} on the CPU"
    );
    assert_eq!(none, 0, "no sub-emitter, no splashes");
    let ratio = gpu as f32 / cpu as f32;
    assert!((0.6..1.6).contains(&ratio), "{gpu} against {cpu}");
}

#[test]
fn a_pool_is_refused_past_the_devices_limits_and_counts_against_the_cap() {
    let world = world();
    // One storage binding holds 128 MiB on WebGPU's default limits (the
    // shared device's, and every tier A device's): 1,677,721 particles.
    assert_eq!(
        world.add_emitter(Emitter::new(2_000_000)),
        Err(EmitterError::TooMany {
            asked: 2_000_000,
            most: 1_677_721
        })
    );
    // The most is made and steps on the device: no error reaches its
    // handler (which panics in debug builds).
    world
        .add_emitter(Emitter::new(1_677_721).rate(100_000.))
        .unwrap();
    if let Some(mut frames) = Frames::new(&world) {
        frames.ready(&world, 0.);
        frames.run(0., 0.1);
        frames.pixels();
    }
    // Pools count with the app's buffers and textures under the cap.
    let world = self::world();
    world.set_memory_cap(1 << 20);
    let buffer = world.add_buffer(BufferDesc::bytes(512 << 10)).unwrap();
    assert_eq!(
        world.add_emitter(Emitter::new(10_000)),
        Err(EmitterError::OverBudget {
            asked: 800_000,
            used: 512 << 10,
            cap: 1 << 20
        })
    );
    let rain = world.add_emitter(Emitter::new(5_000)).unwrap();
    assert_eq!(world.memory().particles, 400_000);
    assert!(
        world.add_buffer(BufferDesc::bytes(200_000)).is_err(),
        "the pool's bytes are counted"
    );
    world.remove_emitter(rain);
    assert!(world.add_buffer(BufferDesc::bytes(200_000)).is_ok());
    drop(buffer);
}

#[test]
fn a_pool_follows_the_way_its_particles_are_updated_from_the_next_frame() {
    let world = world();
    world
        .add_emitter(Emitter::new(20_000).rate(10_000.))
        .unwrap();
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    frames.ready(&world, 0.);
    frames.draw(1. / 60.);
    assert_eq!(world.particle_pools(), [(20_000, false)]);
    // On the CPU: its pool is the CPU's, at most CPU_PARTICLES, in the very
    // next frame.
    world.set_particles_on_cpu(true);
    frames.draw(2. / 60.);
    assert_eq!(world.particle_pools(), [(CPU_PARTICLES, true)]);
    world.set_particles_on_cpu(false);
    frames.draw(3. / 60.);
    assert_eq!(world.particle_pools(), [(20_000, false)]);
}

#[test]
fn a_sub_emitters_births_are_bounded_by_its_pool_whatever_count_is_asked() {
    // A thousand-particle child asked for a billion births each death
    // (and u32::MAX each hit): each event and each frame give it at most
    // its slots, on the CPU and the GPU, and frames end promptly.
    for on_cpu in [true, false] {
        let world = world();
        let Some(mut frames) = Frames::new(&world) else {
            return;
        };
        world.set_particles_on_cpu(on_cpu);
        let child = world
            .add_emitter(Emitter::new(1000).lifetime(0.2..0.2))
            .unwrap();
        world
            .add_emitter(
                Emitter::new(100)
                    .burst(100)
                    .lifetime(0.05..0.05)
                    .force(Force::Gravity)
                    .collide(Collide::Plane {
                        height: -0.01,
                        then: Response::Bounce {
                            keep: 0.5,
                            friction: 0.,
                        },
                    })
                    .on_death(child, 1 << 30)
                    .on_hit(child, u32::MAX),
            )
            .unwrap();
        frames.ready(&world, 0.);
        let started = std::time::Instant::now();
        frames.run(0., 0.5);
        frames.pixels();
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_secs(5),
            "{} frames took {took:?}",
            if on_cpu { "the CPU's" } else { "the GPU's" }
        );
    }
}

#[test]
fn an_emitter_whose_pool_the_device_refused_lets_its_world_settle() {
    let world = world();
    // Within WebGPU's default binding (80 MB of 128), so it's added; the
    // device it's drawn on binds 16 MiB at most, so its pool is refused.
    world
        .add_emitter(Emitter::new(1_000_000).rate(1000.).lifetime(0.1..0.1))
        .unwrap();
    let Some(mut frames) = Frames::with_limits(
        &world,
        wgpu::Limits {
            max_storage_buffer_binding_size: 16 << 20,
            ..Default::default()
        },
    ) else {
        return;
    };
    frames.ready(&world, 0.);
    frames.run(0., 0.1);
    assert!(
        !world.view().animating(),
        "a pool that can't be made draws nothing and keeps nothing drawing"
    );
}

#[test]
fn a_worlds_particles_on_the_cpu_are_bounded_however_many_emitters() {
    let world = world();
    world.set_particles_on_cpu(true);
    // Four full pools fill the world's CPU particles; a fifth is refused.
    for _ in 0..4 {
        world.add_emitter(Emitter::new(CPU_PARTICLES)).unwrap();
    }
    assert_eq!(
        world.add_emitter(Emitter::new(10)),
        Err(EmitterError::TooMany { asked: 10, most: 0 })
    );
    assert_eq!(world.memory().particles, 4 * u64::from(CPU_PARTICLES) * 80);
}

#[test]
fn a_scene_built_before_its_first_frame_steps_at_most_the_worlds_cpu_particles() {
    // Built before the first frame says the device has no compute: ten
    // full pools are accepted, then the CPU steps four (65,536 particles)
    // and the rest say they're over the world's CPU particles.
    let world = world();
    let emitters: Vec<EmitterId> = (0..10)
        .map(|_| {
            world
                .add_emitter(Emitter::new(CPU_PARTICLES).rate(1000.))
                .unwrap()
        })
        .collect();
    let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_compute_workgroups_per_dimension: 0,
            max_compute_invocations_per_workgroup: 0,
            ..wgpu::Limits::downlevel_defaults()
        },
        ..Default::default()
    });
    let mut painter = world.viewport();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for frame in 0..200 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut drawing =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, FORMAT, (8, 8), 1.);
        drawing.time = Instant::now() + Duration::from_millis(frame * 16);
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        if world.particles_ready() && frame > 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!world.caps().unwrap().compute);
    assert_eq!(world.particle_pools(), vec![(CPU_PARTICLES, true); 4]);
    let statuses: Vec<_> = emitters.iter().map(|&e| world.emitter_status(e)).collect();
    assert_eq!(statuses[..4], [Some(EmitterStatus::Running); 4]);
    assert_eq!(statuses[4..], [Some(EmitterStatus::OverCpuParticles); 6]);
}

#[test]
fn an_emitter_changes_from_its_next_update_its_particles_kept() {
    let world = world();
    // A burst of long-lived white discs at the middle.
    let id = world
        .add_emitter(
            Emitter::new(16)
                .burst(16)
                .lifetime(10.0..10.0)
                .size(1.0..1.0)
                .color([1., 1., 1., 1.]),
        )
        .unwrap();
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    frames.ready(&world, 0.);
    frames.draw(1. / 60.);
    assert!(pixel(&frames.pixels(), 32, 32)[1] > 200, "white");
    // Red over their life: the same particles turn red the next frame (a
    // pool made again would show none).
    world.set_emitter_color_over_life(id, [[1., 0., 0., 1.]; 4]);
    frames.draw(2. / 60.);
    let [r, g, b, _] = pixel(&frames.pixels(), 32, 32);
    assert!(r > 200 && g < 40 && b < 40, "{:?}", [r, g, b]);
    // A rate from none keeps the world drawing; none again, it settles once
    // they've lived out.
    world.set_emitter_rate(id, 100.);
    assert!(world.view().animating());
    assert_eq!(world.memory().particles, 16 * 80, "the same pool");
}

#[test]
fn a_new_max_makes_the_pool_again_held_to_the_limits() {
    let world = world();
    let id = world.add_emitter(Emitter::new(1000)).unwrap();
    assert_eq!(world.memory().particles, 1000 * 80);
    let mut emitter = world.emitter(id).unwrap();
    // Past the device's binding: refused, the emitter as it was.
    assert_eq!(
        world.set_emitter(id, Emitter::new(2_000_000)),
        Err(EmitterError::TooMany {
            asked: 2_000_000,
            most: 1_677_721
        })
    );
    assert_eq!(world.memory().particles, 1000 * 80);
    // Within: counted in place of the old pool, against the cap.
    world.set_memory_cap(200_000);
    emitter = emitter.rate(5.);
    let bigger = Emitter::new(2000).rate(5.);
    assert!(
        world.set_emitter(id, bigger).is_ok(),
        "160 KB in place of 80 KB"
    );
    assert_eq!(world.memory().particles, 2000 * 80);
    assert!(matches!(
        world.set_emitter(id, Emitter::new(3000)),
        Err(EmitterError::OverBudget { .. })
    ));
    drop(emitter);
}

#[test]
fn values_past_reason_set_on_an_emitter_are_held_to_it() {
    let world = world();
    let id = world.add_emitter(Emitter::new(100).rate(10.)).unwrap();
    world.set_emitter_rate(id, f32::NAN);
    world.set_emitter_lifetime(id, -5.0..-1.0);
    world.set_emitter_size(id, -1.0..f32::INFINITY);
    let emitter = world.emitter(id).unwrap();
    // No rate, so it settles; a lifetime and size it can draw.
    assert!(!world.view().animating());
    assert_eq!(
        world.emitter(id).unwrap(),
        Emitter::new(100)
            .rate(0.)
            .lifetime(-5.0..-1.0)
            .size(-1.0..f32::INFINITY)
    );
    drop(emitter);
    // A life past all reason is an hour's: the world settles after it.
    world.set_emitter_lifetime(id, 1.0..f32::INFINITY);
    world.burst(id, 1);
    assert!(world.view().animating());
}

/// How a frame is made after its meshes are drawn.
#[derive(Clone, Copy, Debug)]
enum Look {
    Plain,
    /// The post chain on, as nothing: a bloom of none.
    #[cfg(feature = "post")]
    Post,
    /// Temporal antialiasing: each frame a fraction of a pixel aside (the
    /// meshes' jittered matrices, which the particles get too), blended
    /// with those before.
    #[cfg(feature = "post")]
    Taa,
}
const LOOKS: &[Look] = &[
    Look::Plain,
    #[cfg(feature = "post")]
    Look::Post,
    #[cfg(feature = "post")]
    Look::Taa,
];

/// The middle pixel of a world with an unlit blue cube at the origin and a
/// red `blend` burst at `z` (the camera at 4: past 0.5 in front of the
/// cube, past -0.5 behind it), made with `look`.
fn middle_with_a_cube(blend: Blend, z: f32, look: Look) -> Option<[u8; 4]> {
    let world = world();
    let mut blue = Material::rgb(0x0000ff);
    blue.unlit = true;
    world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), blue));
    match look {
        Look::Plain => {}
        #[cfg(feature = "post")]
        Look::Post => world.set_post(Post {
            bloom: Some(Bloom { intensity: 0. }),
            ..Default::default()
        }),
        #[cfg(feature = "post")]
        Look::Taa => world.set_antialiasing(Antialiasing::Taa),
    }
    world
        .add_emitter(
            Emitter::new(16)
                .spawn(Spawn::point([0., 0., z]))
                .burst(16)
                .lifetime(10.0..10.0)
                .size(0.5..0.5)
                .color([1., 0., 0., 1.])
                .draw(Shape::Billboard, blend),
        )
        .unwrap();
    let mut frames = Frames::new(&world)?;
    frames.ready(&world, 0.);
    // Temporal antialiasing settles over its 8 frames: a quarter second.
    frames.run(0., 0.25);
    Some(pixel(&frames.pixels(), 32, 32))
}

#[test]
fn an_emitter_behind_a_cube_is_hidden_and_one_in_front_shows_however_the_frame_is_made() {
    for &look in LOOKS {
        for blend in [Blend::Alpha, Blend::Additive] {
            let Some(behind) = middle_with_a_cube(blend, -1.5, look) else {
                return;
            };
            assert_eq!(behind, [0, 0, 255, 255], "{blend:?} behind, {look:?}");
            let front = middle_with_a_cube(blend, 1.5, look).unwrap();
            // Alpha: red over the blue; additive: red added to it.
            let expected = match blend {
                Blend::Alpha => [255, 0, 0, 255],
                Blend::Additive => [255, 0, 255, 255],
            };
            assert_eq!(front, expected, "{blend:?} in front, {look:?}");
        }
    }
}

#[cfg(feature = "post")]
#[test]
fn a_bright_spark_blooms_with_the_scene() {
    // A small additive spark, eight times white, before a black wall.
    let lit = |bloom: Option<Bloom>| {
        let world = world();
        let mut black = Material::rgb(0x000000);
        black.unlit = true;
        world.add(Object::mesh(Geometry::cuboid(20., 20., 0.1), black).at([0., 0., -3.]));
        world.set_post(Post {
            bloom,
            ..Default::default()
        });
        world
            .add_emitter(
                Emitter::new(1)
                    .burst(1)
                    .lifetime(10.0..10.0)
                    .size(0.2..0.2)
                    .color([8., 8., 8., 1.])
                    .draw(Shape::Billboard, Blend::Additive),
            )
            .unwrap();
        let mut frames = Frames::new(&world)?;
        frames.ready(&world, 0.);
        frames.draw(1. / 60.);
        let pixels = frames.pixels();
        // Pixels any light reaches.
        Some(
            (0..SIDE as usize * SIDE as usize)
                .filter(|i| pixel(&pixels, i % SIDE as usize, i / SIDE as usize)[0] > 4)
                .count(),
        )
    };
    let Some(alone) = lit(None) else {
        return;
    };
    let bloomed = lit(Some(Bloom { intensity: 0.3 })).unwrap();
    assert!(alone > 0, "the spark shows");
    // Its glare spreads well past it.
    assert!(bloomed > alone * 3, "{alone} lit alone, {bloomed} bloomed");
}

/// `world` (a burst of discs living a second at its middle) drawn by
/// `frames` to 0.6 s of frame time: whether they still show in each
/// painter's target (at double the world's rate they'd have died).
fn alive_at_six_tenths(world: &World, frames: &mut Frames) -> (bool, Option<bool>) {
    frames.ready(world, 0.);
    frames.run(0., 0.6);
    let lit = |pixels: Vec<u8>| pixel(&pixels, 32, 32)[0] > 200;
    let second = frames.second.is_some().then(|| lit(frames.second_pixels()));
    (lit(frames.pixels()), second)
}

fn a_seconds_burst(world: &World) {
    world
        .add_emitter(
            Emitter::new(16)
                .burst(16)
                .lifetime(1.0..1.0)
                .size(1.0..1.0)
                .color([1., 0., 0., 1.]),
        )
        .unwrap();
}

#[test]
fn a_world_shown_in_two_boxes_steps_its_particles_once_a_frame() {
    let world = world();
    a_seconds_burst(&world);
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    frames.boxes = 2;
    assert_eq!(alive_at_six_tenths(&world, &mut frames), (true, None));
}

#[test]
fn two_painters_of_one_world_step_its_particles_once_a_frame_and_both_draw_them() {
    let world = world();
    a_seconds_burst(&world);
    let Some(frames) = Frames::new(&world) else {
        return;
    };
    let mut frames = frames.with_second(&world);
    assert_eq!(
        alive_at_six_tenths(&world, &mut frames),
        (true, Some(true)),
        "alive at the world's rate, and drawn by both"
    );
}

/// Through a real renderer, which reads the interface's clock once a window
/// frame (here a virtual one, moved by the test): a world's view in two
/// boxes of a window steps its particles once a frame, and both boxes show
/// them.
#[test]
fn a_world_shown_in_two_boxes_of_a_window_steps_its_particles_once_a_frame() {
    use zgx::{compose::prelude::*, widgets::Ui};
    let clock = zgx::clock::VirtualClock::new();
    let world = world();
    a_seconds_burst(&world);
    let mut gpu = match zgx_gpu::GpuRenderer::new(2 * SIDE, SIDE) {
        Ok(gpu) => gpu,
        Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
            panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
        }
        Err(_) => return,
    };
    let side = SIDE as f32;
    let mut ui = Ui::new(2. * side, side);
    ui.mount(
        div()
            .size(2. * side, side)
            .flex_row()
            .child(gpu_view("World", world.view()).size(side, side))
            .child(gpu_view("World", world.view()).size(side, side)),
    );
    let mut draw = || {
        world.view().invalidate();
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    };
    // Frames at the clock's start until the pipelines are made (up to 10 s
    // of wall time: compiling isn't what's tested).
    let started = Instant::now();
    while !world.particles_ready() {
        assert!(started.elapsed() < Duration::from_secs(10), "not made");
        draw();
        std::thread::sleep(Duration::from_millis(2));
    }
    // To 0.6 s at 60 frames a second, each frame drawing the view twice:
    // stepped twice a frame, the second-long burst would have died.
    for frame in 1..=36u64 {
        clock.advance(Duration::from_millis(frame * 50 / 3), || {});
        draw();
    }
    let pixels = gpu.readback().unwrap();
    let lit = |x: u32| pixels[((SIDE / 2 * 2 * SIDE + x) * 4) as usize] > 200;
    assert!(lit(SIDE / 2) && lit(SIDE + SIDE / 2), "alive in both boxes");
}

#[test]
fn a_painter_still_compiling_that_draws_first_costs_the_world_no_time() {
    let world = world();
    a_seconds_burst(&world);
    let Some(mut frames) = Frames::new(&world) else {
        return;
    };
    frames.ready(&world, 0.);
    let at = frames.run(0., 0.25);
    let before = world.particle_time();
    // A second painter, drawing first each frame: its renderer prepares
    // the drawer only as it draws, so on its first frame it can't step the
    // world; the painter after it at that frame time does.
    frames = frames.with_second(&world);
    frames.second_first = true;
    frames.draw(at + 1. / 60.);
    let stepped = world.particle_time() - before;
    assert!(
        (stepped - 1. / 60.).abs() < 1e-6,
        "{stepped} s stepped in a frame of 1/60"
    );
}

#[cfg(feature = "post")]
#[test]
fn a_fast_particle_under_temporal_antialiasing_keeps_its_light() {
    // One white particle crossing the view at 4 m/s: under temporal
    // antialiasing its frames weigh as a moving mesh's (half at least), so
    // it isn't pulled toward the background, its brightest pixel near
    // multisampling's.
    let brightest = |antialiasing| {
        let world = world();
        world.set_antialiasing(antialiasing);
        world
            .add_emitter(
                Emitter::new(1)
                    .spawn(Spawn::point([-1.2, 0., 0.]))
                    .burst(1)
                    .lifetime(5.0..5.0)
                    .velocity([4., 0., 0.], 0.)
                    .size(0.15..0.15)
                    .color([1., 1., 1., 1.]),
            )
            .unwrap();
        let mut frames = Frames::new(&world)?;
        frames.ready(&world, 0.);
        for frame in 1..=10 {
            frames.draw(f64::from(frame) / 60.);
        }
        let pixels = frames.pixels();
        Some(pixels.chunks(4).map(|p| p[2]).max().unwrap_or(0))
    };
    let Some(msaa) = brightest(Antialiasing::Msaa) else {
        return;
    };
    let taa = brightest(Antialiasing::Taa).unwrap();
    assert!(msaa > 200, "drawn: {msaa}");
    // Half its frame (until the prepass's velocity): 225 of 255 here, 121
    // when it blended as a still frame.
    assert!(
        u16::from(taa) + 40 >= u16::from(msaa),
        "TAA {taa}, multisampling {msaa}"
    );
}

/// Devices of two wgpu instances compare equal (their ids are the same
/// numbers), so a world drawn on both must tell them apart by identity
/// (`ViewportFrame::device_id`): each keeps its own pools, pipelines and
/// buffers, the world still steps once a frame time, and a device made
/// after one is gone takes nothing of it.
#[test]
fn a_world_drawn_on_devices_of_two_instances_keeps_each_its_own() {
    use zgx::{compose::prelude::*, gpu_view::GpuView, widgets::Ui};
    let clock = zgx::clock::VirtualClock::new();
    let world = world();
    // Spawning on: a device made later has new pools, nothing of what
    // came before, and fills them.
    world
        .add_emitter(
            Emitter::new(64)
                .rate(60.)
                .lifetime(1.0..1.0)
                .size(1.0..1.0)
                .color([1., 0., 0., 1.]),
        )
        .unwrap();
    // And the app's GPU data, with a pass using it each frame.
    let count = world.add_buffer(BufferDesc::storage::<u32>(1)).unwrap();
    world
        .add_compute(
            Compute::new(
                "@group(0) @binding(0) var<storage, read_write> count: array<u32>;
                 @compute @workgroup_size(1) fn main() { count[0] += 1u; }",
            )
            .buffer("count", &count, Access::ReadWrite)
            .every_frame(),
        )
        .unwrap();
    // A renderer (a context, a wgpu instance and a device of its own)
    // showing `view`.
    let window = |view: &GpuView| {
        let gpu = match zgx_gpu::GpuRenderer::new(SIDE, SIDE) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let side = SIDE as f32;
        let mut ui = Ui::new(side, side);
        ui.mount(gpu_view("World", view).size(side, side));
        Some((ui, gpu))
    };
    // The world's own view and a second painter of it, each in a window.
    let second_view = zgx_gpu::viewport::view(world.viewport());
    let draw = |(ui, gpu): &mut (Ui, zgx_gpu::GpuRenderer)| {
        world.view().invalidate();
        second_view.invalidate();
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
    };
    let lit = |(_, gpu): &mut (Ui, zgx_gpu::GpuRenderer)| {
        let pixels = gpu.readback().unwrap();
        pixels[((SIDE / 2 * SIDE + SIDE / 2) * 4) as usize] > 200
    };
    let Some(mut first) = window(world.view()) else {
        return;
    };
    let mut second = window(&second_view).unwrap();
    let started = Instant::now();
    let mut frame = 0u64;
    let both = |first: &mut _, second: &mut _, frame: &mut u64| {
        *frame += 1;
        clock.advance(Duration::from_micros(*frame * 16_667), || {});
        draw(first);
        draw(second);
    };
    while !(world.particles_ready() && lit(&mut first) && lit(&mut second)) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "not drawn: ready {}, first {}, second {}, pools {:?}",
            world.particles_ready(),
            lit(&mut first),
            lit(&mut second),
            world.particle_pools()
        );
        both(&mut first, &mut second, &mut frame);
        std::thread::sleep(Duration::from_millis(2));
    }
    // Once a frame time, whichever device draws first.
    let before = world.particle_time();
    for _ in 0..30 {
        both(&mut first, &mut second, &mut frame);
    }
    let stepped = world.particle_time() - before;
    assert!((stepped - 0.5).abs() < 1e-3, "{stepped} s in 30 frames");
    assert!(
        lit(&mut first) && lit(&mut second),
        "each device draws them"
    );
    // One gone, a third made: it takes nothing of the gone one's.
    drop(first);
    let Some(mut third) = window(world.view()) else {
        return;
    };
    let started = Instant::now();
    while !(lit(&mut third) && lit(&mut second)) {
        assert!(started.elapsed() < Duration::from_secs(10), "not drawn");
        both(&mut third, &mut second, &mut frame);
        std::thread::sleep(Duration::from_millis(2));
    }
}
