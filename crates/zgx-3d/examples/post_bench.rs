//! What the post chain costs a frame on the GPU (design-3d-aaa.md, 11):
//! a lit scene drawn headless at `SIZE` (1920x1080 by default), the camera
//! moving a little each frame, under arms of the chain interleaved in
//! `ROUNDS` rounds of `FRAMES` frames, each frame waited for (its pixels
//! read back: the same cost in every arm). Prints each arm's median frame
//! and its difference from the arm without the chain; each arm's frames
//! drawn are counted, and arms that should look different are checked to.
//! Run: cargo run --release -p zgx-3d --features post --example post_bench
use std::time::Instant;
use zgx::{compose::prelude::*, widgets::Ui};
use zgx_3d::*;
use zgx_gpu::GpuRenderer;

fn env(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let (width, height) = std::env::var("SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1920, 1080));
    let (rounds, frames) = (env("ROUNDS", 7), env("FRAMES", 30));
    let world = World::new();
    for i in 0..200 {
        let (x, z) = ((i % 20) as f32 - 9.5, (i / 20) as f32 - 4.5);
        let material = Material::rgb(0x40_80_c0 + ((i as u32 * 0x0a0503) & 0x3f3f3f));
        world.add(Object::mesh(Geometry::sphere(0.4, 32), material).at([x, 0., z]));
    }
    world.add_light(Light::Directional {
        direction: vec3(-0.4, -1., -0.6),
        color: [1., 1., 1.],
        intensity: 6.,
    });
    world.set_background(Some(0x101418));
    let mut gpu = GpuRenderer::new(width, height).expect("a GPU");
    let mut ui = Ui::new(width as f32, height as f32);
    ui.mount(gpu_view("World", world.view()).size(width as f32, height as f32));
    let arms: [(&str, Post); 4] = [
        ("off", Post::default()),
        (
            "chain",
            Post {
                vignette: Some(Vignette { intensity: 0. }),
                ..Default::default()
            },
        ),
        (
            "bloom",
            Post {
                bloom: Some(Bloom::default()),
                ..Default::default()
            },
        ),
        (
            "all",
            Post {
                bloom: Some(Bloom::default()),
                vignette: Some(Vignette::default()),
                grain: Some(Grain::default()),
                chromatic_aberration: Some(ChromaticAberration::default()),
                grade: Some(Grade {
                    temperature: 0.1,
                    saturation: 1.1,
                    lut: Some(Lut::identity(32)),
                    ..Default::default()
                }),
                auto_exposure: Some(AutoExposure::default()),
            },
        ),
    ];
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); arms.len()];
    let mut last: Vec<Vec<u8>> = vec![Vec::new(); arms.len()];
    let mut step = 0u32;
    for round in 0..=rounds {
        for (arm, (_, post)) in arms.iter().enumerate() {
            world.set_post(post.clone());
            let drawn = world.frames_drawn();
            for frame in 0..=frames {
                step += 1;
                let angle = step as f32 * 0.002;
                world.set_camera(Camera::looking_at(
                    [9. * angle.sin(), 5., 9. * angle.cos()],
                    [0., 0., 0.],
                ));
                let start = Instant::now();
                ui.prepare_frame();
                let report = ui.scene.borrow_mut().flush();
                gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
                let pixels = gpu.readback().unwrap();
                let took = start.elapsed().as_secs_f64() * 1e3;
                // Round 0 warms up; each arm's first frame makes its targets.
                if round > 0 && frame > 0 {
                    times[arm].push(took);
                }
                last[arm] = pixels;
            }
            assert_eq!(
                world.frames_drawn() - drawn,
                u64::from(frames) + 1,
                "every frame of {} drew",
                arms[arm].0
            );
        }
    }
    assert_ne!(last[0], last[2], "bloom shows");
    let median = |t: &mut Vec<f64>| {
        t.sort_by(f64::total_cmp);
        t[t.len() / 2]
    };
    let medians: Vec<f64> = times.iter_mut().map(median).collect();
    println!("{width}x{height}, {rounds} rounds of {frames} frames, median ms a frame:");
    for ((name, _), m) in arms.iter().zip(&medians) {
        println!("{name:>6} {m:8.3}  {:+.3}", m - medians[0]);
    }
}
