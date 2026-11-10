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
}
impl Frames {
    fn new(world: &World) -> Option<Self> {
        Self::with_limits(world, wgpu::Limits::default())
    }
    /// On a device asked for `limits`.
    fn with_limits(world: &World, limits: wgpu::Limits) -> Option<Self> {
        let (device, queue) = device(limits)?;
        let target = device.create_texture(&wgpu::TextureDescriptor {
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
        });
        Some(Self {
            device,
            queue,
            target,
            painter: Box::new(world.viewport()),
            start: Instant::now(),
        })
    }
    /// A frame at `seconds` of frame time.
    fn draw(&mut self, seconds: f64) {
        let view = self.target.create_view(&Default::default());
        let srgb = self.target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(FORMAT.add_srgb_suffix()),
            ..Default::default()
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut frame = ViewportFrame::new(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            FORMAT,
            (SIDE, SIDE),
            1.,
        );
        frame.target_srgb = Some(&srgb);
        frame.time = self.start + Duration::from_secs_f64(seconds);
        self.painter.render(&mut frame);
        self.queue.submit([encoder.finish()]);
    }
    /// Frames at `from` until the world's particles' pipelines are made
    /// (on their thread): its particles step from the next.
    fn ready(&mut self, world: &World, from: f64) {
        for _ in 0..1000 {
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
    /// The target's pixels, BGRA.
    fn pixels(&self) -> Vec<u8> {
        let row = (SIDE * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            self.target.size(),
        );
        self.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        (0..SIDE as usize)
            .flat_map(|y| mapped[y * row as usize..][..SIDE as usize * 4].to_vec())
            .collect()
    }
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
