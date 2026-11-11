//! What a world's sound costs a frame on the main thread, by its meshes: 8
//! emitters with occlusion, the camera walking (so every emitter's rays are
//! cast again), over N small meshes, off the rays' paths or across them;
//! and the same with one mesh moving too, so the world's meshes changed.
//!
//! `cargo run --release -p zgx-3d --features audio --example audio_occlusion`
use std::time::{Duration, Instant};
use zgx_3d::{Camera, Emitter3d, Geometry, Material, Object, Transform, World};
use zgx_audio::{Audio, Config, Sound};

fn frame_cost(meshes: usize, across: bool, moving_mesh: bool) -> Duration {
    let (audio, mut mixer) = Audio::offline(Config::default()).unwrap();
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 0.], [0., 0., -1.]));
    world
        .set_audio(&audio, &audio.bus("world").unwrap())
        .unwrap();
    let mut ids = Vec::new();
    for i in 0..meshes {
        let id = world.add(Object::mesh(
            Geometry::cuboid(0.3, 0.3, 0.3),
            Material::default(),
        ));
        // A wall of them 3 m ahead (across the rays), or high above.
        let (x, y) = ((i % 100) as f32 * 0.4 - 20., (i / 100) as f32 * 0.4);
        let at = if across {
            [x, y - 4., -3.]
        } else {
            [x, y + 30., -3.]
        };
        world.set_transform(id, Transform::at(at));
        ids.push(id);
    }
    let tone = Sound::from_samples(&[0., 0.5], 1, 48_000).unwrap();
    for i in 0..8 {
        let id = world.add(Object::group());
        world.set_transform(id, Transform::at([i as f32 - 4., 0., -6.]));
        world
            .add_emitter3d(id, Emitter3d::new(&tone).looping(true))
            .unwrap();
    }
    let start = Instant::now();
    world.update_audio(start);
    let mut best = Duration::MAX;
    for k in 1..40u32 {
        world.set_camera(Camera::looking_at(
            [0.01 * k as f32, 0., 0.],
            [0.01 * k as f32, 0., -1.],
        ));
        if moving_mesh {
            world.set_transform(ids[0], Transform::at([0.001 * k as f32, 30., -3.]));
        }
        let began = Instant::now();
        world.update_audio(start + Duration::from_millis(10 * u64::from(k)));
        best = best.min(began.elapsed());
        mixer.frames(480);
    }
    best
}

fn main() {
    println!("meshes  off the rays  across them  a mesh moving");
    for meshes in [500, 1_000, 2_000, 4_000] {
        println!(
            "{meshes:>6}  {:>10.1?}  {:>10.1?}  {:>12.1?}",
            frame_cost(meshes, false, false),
            frame_cost(meshes, true, false),
            frame_cost(meshes, false, true),
        );
    }
}
