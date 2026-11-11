//! The animation comparison's zgx arm (`comparisons/3d/README.md`,
//! "Animation"): `INSTANCES` copies of the glTF file `MODEL` placed in a
//! row, each with an animator playing its first clip (`ARM=play`) or
//! standing still (`ARM=still`), drawn into an offscreen 256² target on a
//! Metal device every frame, frames a fixed 1/60 s apart. Prints the
//! process's instructions and CPU time a frame over `FRAMES` frames after
//! 60 to warm up: `play` less `still` is what playing costs. The Bevy arm
//! (`comparisons/3d/bevy-anim`) does the same.
#[path = "../../../common/rusage.rs"]
mod rusage;

use std::time::{Duration, Instant};
use zgx_3d::*;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let path = std::env::var("MODEL").expect("MODEL: a .glb");
    let model = Model::from_bytes(&std::fs::read(&path).expect("the model")).expect("a model");
    let clip = model.clips().first().expect("a clip").clone();
    let (instances, frames) = (setting("INSTANCES", 100), setting("FRAMES", 300));
    let play = std::env::var("ARM").map_or(true, |arm| arm == "play");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let world = World::new();
    world.add_light(Light::Directional {
        direction: vec3(-0.5, -1., -0.4),
        color: [1.; 3],
        intensity: 2.5,
    });
    world.set_camera(Camera::looking_at([0., 2., 30.], [0., 1., 0.]));
    let side = (instances as f32).sqrt().ceil() as usize;
    for i in 0..instances {
        let placed = world.place_model(&model);
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        world.set_transform(
            placed.root,
            Transform::at([x * 1.5 - side as f32 * 0.75, 0., -z * 1.5]),
        );
        if play {
            let animator = world.add_animator(&placed);
            world.play(animator, &clip, Play::new()).expect("layer 0");
        }
    }
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[FORMAT.add_srgb_suffix()],
    });
    let view = target.create_view(&Default::default());
    let srgb = target.create_view(&wgpu::TextureViewDescriptor {
        format: Some(FORMAT.add_srgb_suffix()),
        ..Default::default()
    });
    let mut painter = world.viewport();
    let start = Instant::now();
    let mut frame = |n: usize| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut drawing =
            ViewportFrame::new(&device, &queue, &mut encoder, &view, FORMAT, (256, 256), 1.);
        drawing.target_srgb = Some(&srgb);
        drawing.time = start + Duration::from_secs_f64(n as f64 / 60.);
        painter.render(&mut drawing);
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    for n in 0..60 {
        frame(n);
    }
    let before = rusage::sample();
    for n in 60..60 + frames {
        frame(n);
    }
    let after = rusage::sample();
    // It did the work: every frame of `play` was drawn by its animators.
    assert_eq!(world.view().animating(), play);
    println!(
        "zgx arm={} instances={instances} frames={frames} drawn={} instructions/frame={:.0} cpu_us/frame={:.1}",
        if play { "play" } else { "still" },
        world.frames_drawn(),
        (after.instructions - before.instructions) as f64 / frames as f64,
        (after.cpu_ns - before.cpu_ns) / frames as f64 / 1e3,
    );
}
