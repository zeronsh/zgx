//! GPU time of particles at 1080p on this machine's GPU: frames of a world
//! with only a background (the control), and of the same world raining,
//! submitted back to back and waited for, arms interleaved. A frame's GPU
//! time is the arms' difference in time per frame. Each arm says how many
//! of its frames' pixels differ from the background, to show it drew.
//!
//! Run: cargo run --release -p zgx-3d --features particles --example particles_bench
//! (`ROUNDS`, `FRAMES`; `ALIVE`: the rain's particles alive at once.)
use std::time::{Duration, Instant};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const SIZE: (u32, u32) = (1920, 1080);

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Arm {
    name: &'static str,
    world: World,
    painter: Box<dyn Viewport>,
    frame: usize,
}

fn world(alive: Option<u32>) -> World {
    let world = World::new();
    world.set_background(Some(0x0b0e14));
    world.set_camera(Camera::looking_at([0., 1.7, 0.], [0., 1.5, -10.]));
    if let Some(alive) = alive {
        // As design-3d-aaa.md's rain: streaks over 80 by 80 metres, falling
        // from 30 metres, living 0.6 to 0.9 s; the rate keeps `alive` of
        // them at once.
        world
            .add_emitter(
                Emitter::new(alive)
                    .spawn(
                        Spawn::box_([-40., 28., -40.], [40., 32., 40.]).rate(alive as f32 / 0.75),
                    )
                    .lifetime(0.6..0.9)
                    .velocity([0., -22., 0.], 1.5)
                    .force(Force::Gravity)
                    .force(Force::Wind([2., 0., 1.]))
                    .force(Force::Drag(0.1))
                    .collide(Collide::Plane {
                        height: 0.,
                        then: Response::Die,
                    })
                    .draw(Shape::Streak { length: 0.04 }, Blend::Alpha)
                    .size(0.01..0.015)
                    .color([0.7, 0.75, 0.8, 0.5]),
            )
            .expect("an emitter");
    }
    world
}

fn main() {
    let (rounds, frames) = (setting("ROUNDS", 5), setting("FRAMES", 240));
    let alive = setting("ALIVE", 200_000) as u32;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block(instance.request_adapter(&Default::default())).expect("an adapter");
    println!(
        "{:?} on {}",
        adapter.get_info().backend,
        adapter.get_info().name
    );
    let (device, queue) = block(adapter.request_device(&Default::default())).expect("a device");
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
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
    let start = Instant::now();
    let mut arms: Vec<Arm> = [("empty", None), ("rain", Some(alive))]
        .into_iter()
        .map(|(name, alive)| {
            let world = world(alive);
            Arm {
                name,
                painter: Box::new(world.viewport()),
                world,
                frame: 0,
            }
        })
        .collect();
    let draw = |arm: &mut Arm| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame = ViewportFrame::new(&device, &queue, &mut encoder, &view, FORMAT, SIZE, 1.);
        frame.target_srgb = Some(&srgb);
        frame.time = start + Duration::from_micros(arm.frame as u64 * 16_667);
        arm.frame += 1;
        arm.painter.render(&mut frame);
        queue.submit([encoder.finish()]);
    };
    // Pipelines made, then a second of rain so as many live as will.
    for arm in &mut arms {
        draw(arm);
        while arm.name == "rain" && !arm.world.particles_ready() {
            std::thread::sleep(Duration::from_millis(10));
            draw(arm);
        }
        for _ in 0..60 {
            draw(arm);
        }
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    // The background as the control drew it.
    draw(&mut arms[0]);
    let background = pixels(&device, &queue, &target)[..4].to_vec();
    for _ in 0..rounds {
        for (at, arm) in arms.iter_mut().enumerate() {
            let began = Instant::now();
            for _ in 0..frames {
                draw(arm);
            }
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            times[at].push(began.elapsed().as_secs_f64() * 1000. / frames as f64);
            let drawn = pixels(&device, &queue, &target)
                .chunks(4)
                .filter(|p| p.iter().zip(&background).any(|(a, b)| a.abs_diff(*b) > 2))
                .count();
            println!(
                "  {}: {:.3} ms a frame, {drawn} pixels drawn over",
                arm.name,
                times[at].last().unwrap()
            );
        }
    }
    let median = |t: &mut Vec<f64>| {
        t.sort_by(f64::total_cmp);
        t[t.len() / 2]
    };
    let (empty, rain) = (median(&mut times[0]), median(&mut times[1]));
    let spread = |t: &[f64]| (t[0], t[t.len() - 1]);
    println!(
        "median: empty {empty:.3} ms {:?}, rain {rain:.3} ms {:?}: {alive} particles cost {:.3} ms a frame",
        spread(&times[0]),
        spread(&times[1]),
        rain - empty
    );
}

/// `target`'s pixels.
fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    let row = SIZE.0 * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * SIZE.1),
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
    mapped.to_vec()
}
