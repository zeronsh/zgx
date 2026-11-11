//! A world's sound, rendered offline: an emitter passing the listener rises
//! and falls in pitch and crosses from left to right; a wall between them
//! dulls its highs; a still world sends nothing; the occlusion rays keep to
//! their budget; and the listener walking from one reverb zone into another
//! hears one fade into the other. Each frame is `World::update_audio` at a
//! time 10 ms on from the last, then 10 ms rendered.
use std::time::{Duration, Instant};
use zgx_3d::{
    Camera, Emitter3d, Geometry, Material, Object, ObjectId, ReverbPreset, ReverbZone, Transform,
    World,
};
use zgx_audio::spatial::{Falloff, doppler};
use zgx_audio::{Audio, Config, Offline, Sound};

const RATE: u32 = 48_000;
/// A frame: 10 ms.
const FRAME: usize = 480;

fn engine() -> (Audio, Offline) {
    Audio::offline(Config {
        limiter: None,
        ..Config::default()
    })
    .unwrap()
}

/// A tone of `hz` (a whole number of cycles in a second), looped.
fn tone(hz: f64) -> Sound {
    let samples: Vec<f32> = (0..RATE as usize)
        .map(|i| ((2. * std::f64::consts::PI * hz * i as f64 / f64::from(RATE)).sin() * 0.5) as f32)
        .collect();
    Sound::from_samples(&samples, 1, RATE).unwrap()
}

/// A world listening from the origin, looking down -z, with sound.
fn world(audio: &Audio) -> World {
    let world = World::new();
    world.set_camera(Camera::looking_at([0., 0., 0.], [0., 0., -1.]));
    world
        .set_audio(audio, &audio.bus("world").unwrap())
        .unwrap();
    world
}

/// An object to put an emitter on: nothing drawn, at `at`.
fn spot(world: &World, at: [f32; 3]) -> ObjectId {
    let id = world.add(Object::group());
    world.set_transform(id, Transform::at(at));
    id
}

/// A clock of frames: the time of frame `k`.
struct Clock(Instant);
impl Clock {
    fn at(&self, k: usize) -> Instant {
        self.0 + Duration::from_millis(10 * k as u64)
    }
}

fn channel(samples: &[f32], c: usize) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[c]).collect()
}

fn level_db(samples: &[f32]) -> f64 {
    let e: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
    10. * (e / samples.len() as f64).log10()
}

/// Rising zero crossings over the time between the first and the last.
fn frequency(samples: &[f32]) -> f64 {
    let rising: Vec<f64> = (1..samples.len())
        .filter(|&i| samples[i - 1] < 0. && samples[i] >= 0.)
        .map(|i| {
            let (a, b) = (f64::from(samples[i - 1]), f64::from(samples[i]));
            (i - 1) as f64 + a / (a - b)
        })
        .collect();
    (rising.len() - 1) as f64 * f64::from(RATE) / (rising[rising.len() - 1] - rising[0])
}

#[test]
fn an_emitter_passing_the_listener_rises_and_falls_and_crosses_from_left_to_right() {
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    // 4 m ahead, from 40 m left to 40 m right at 40 m/s.
    let at = |k: usize| [-40. + 40. * k as f32 * 0.01, 0., -4.];
    let car = spot(&world, at(0));
    world
        .add_emitter3d(
            car,
            Emitter3d::new(&tone(400.))
                .looping(true)
                .falloff(Falloff::inverse(2., 100.))
                .air(false)
                .occlusion(false),
        )
        .unwrap();
    let clock = Clock(Instant::now());
    world.update_audio(clock.at(0));
    let mut out = Vec::new();
    for k in 1..=200 {
        world.set_transform(car, Transform::at(at(k)));
        world.update_audio(clock.at(k));
        out.extend(mixer.frames(FRAME));
    }
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    // Coming, then going: each window's pitch is Doppler's where it was.
    let still = [0.; 3];
    for frame in [20usize, 170] {
        let window = &left[frame * FRAME..(frame + 5) * FRAME];
        let middle = at(frame + 3);
        let want = 400. * f64::from(doppler(middle, [40., 0., 0.], still, still, 1.));
        let got = frequency(window);
        assert!(
            ((got - want) / want).abs() < 0.005,
            "frame {frame}: {got:.2} Hz, not {want:.2}"
        );
    }
    assert!(frequency(&left[20 * FRAME..25 * FRAME]) > 420.);
    assert!(frequency(&left[170 * FRAME..175 * FRAME]) < 380.);
    // Left first, right after.
    let span = |c: &[f32], frame: usize| level_db(&c[frame * FRAME..(frame + 5) * FRAME]);
    assert!(span(&left, 20) > span(&right, 20) + 10., "left first");
    assert!(span(&right, 170) > span(&left, 170) + 10., "right after");
}

#[test]
fn a_wall_between_dulls_the_highs_more_than_the_lows() {
    // A tone 8 m ahead, then a wall put up halfway: the low tone loses the
    // wall's gain (a quarter, -12 dB), the high one that and its low-pass.
    let drop = |hz: f64, transmission: Option<f32>| -> f64 {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let source = spot(&world, [0., 0., -8.]);
        world
            .add_emitter3d(source, Emitter3d::new(&tone(hz)).looping(true).air(false))
            .unwrap();
        let clock = Clock(Instant::now());
        world.update_audio(clock.at(0));
        let open = level_db(&channel(&mixer.frames(RATE as usize / 4), 0)[RATE as usize / 8..]);
        let wall = world.add(Object::mesh(
            Geometry::cuboid(4., 4., 0.2),
            Material::rgb(0x808080),
        ));
        world.set_transform(wall, Transform::at([0., 0., -4.]));
        if let Some(t) = transmission {
            world.set_sound_transmission(wall, t);
        }
        world.update_audio(clock.at(1));
        assert_eq!(world.audio_rays(), 1, "the wall rayed for");
        let walled = level_db(&channel(&mixer.frames(RATE as usize / 4), 0)[RATE as usize / 8..]);
        open - walled
    };
    let (low, high) = (drop(200., None), drop(8_000., None));
    assert!((low - 12.04).abs() < 0.5, "the low tone lost {low:.2} dB");
    assert!(
        high > low + 10.,
        "the high tone lost {high:.2} dB, the low {low:.2}"
    );
    // A wall that lets everything through takes nothing.
    let glass = drop(8_000., Some(1.));
    assert!(glass.abs() < 0.1, "through glass: {glass:.2} dB");
}

#[test]
fn a_still_world_sends_nothing() {
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    for x in [-3., 0., 3.] {
        let id = spot(&world, [x, 0., -5.]);
        world
            .add_emitter3d(id, Emitter3d::new(&tone(400.)).looping(true))
            .unwrap();
    }
    let clock = Clock(Instant::now());
    for k in 0..3 {
        world.update_audio(clock.at(k));
        mixer.frames(FRAME);
    }
    let sent = audio.stats().commands;
    for k in 3..10 {
        world.update_audio(clock.at(k));
        mixer.frames(FRAME);
    }
    assert_eq!(audio.stats().commands, sent, "a still world sent commands");
    assert_eq!(world.audio_rays(), 0);
    // The camera turning is a change: each emitter's pan follows.
    world.set_camera(Camera::looking_at([0., 0., 0.], [1., 0., -1.]));
    world.update_audio(clock.at(10));
    assert!(audio.stats().commands > sent);
}

#[test]
fn occlusion_rays_keep_to_their_budget_and_follow_what_moved() {
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    world.set_occlusion_rays(4);
    let ids: Vec<ObjectId> = (0..20)
        .map(|i| {
            let id = spot(&world, [i as f32 - 10., 0., -6.]);
            world
                .add_emitter3d(id, Emitter3d::new(&tone(400.)).looping(true))
                .unwrap();
            id
        })
        .collect();
    let clock = Clock(Instant::now());
    // 20 to ray, 4 a frame: five frames, then none while nothing moves.
    let cast: Vec<usize> = (0..7)
        .map(|k| {
            world.update_audio(clock.at(k));
            mixer.frames(FRAME);
            world.audio_rays()
        })
        .collect();
    assert_eq!(cast, [4, 4, 4, 4, 4, 0, 0]);
    // One emitter moves: one ray.
    world.set_transform(ids[7], Transform::at([-3., 1., -6.]));
    world.update_audio(clock.at(7));
    assert!(world.audio_rays() >= 1 && world.audio_rays() <= 4);
}

#[test]
fn walking_from_one_reverb_zone_into_another_fades_one_into_the_other() {
    // A dry room on the left, a warehouse on the right, 4 m apart, each
    // fading over 4 m. Clicks played beside the listener: the reverb
    // heard after each grows from nothing as it walks right.
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    // The same reverb either side, only the left's unheard: halfway, the
    // listener hears the right zone half, and the emitter, half in it,
    // sends three quarters: 0.375 of the level, a seventh of the energy.
    let dry = ReverbPreset {
        wet: 0.,
        ..ReverbPreset::WAREHOUSE
    };
    world
        .add_reverb_zone(
            ReverbZone::box_([-12., -5., -12.], [-2., 5., 12.])
                .preset(dry)
                .fade(4.),
        )
        .unwrap();
    world
        .add_reverb_zone(
            ReverbZone::box_([2., -5., -12.], [12., 5., 12.])
                .preset(ReverbPreset::WAREHOUSE)
                .fade(4.),
        )
        .unwrap();
    let mut click = vec![0f32; RATE as usize / 2];
    click[1] = 0.5;
    let click = Sound::from_samples(&click, 1, RATE).unwrap();
    // Heard from the dry room from the start: added where it's heard, it
    // starts with that place's sends.
    world.set_camera(Camera::looking_at([-6., 0., 0.], [-6., 0., -1.]));
    let beside = spot(&world, [-6., 0., -1.]);
    world
        .add_emitter3d(
            beside,
            Emitter3d::new(&click).looping(true).occlusion(false),
        )
        .unwrap();
    let clock = Clock(Instant::now());
    let mut k = 0;
    let mut tails = Vec::new();
    for x in [-6f32, 0., 6.] {
        world.set_camera(Camera::looking_at([x, 0., 0.], [x, 0., -1.]));
        world.set_transform(beside, Transform::at([x, 0., -1.]));
        world.update_audio(clock.at(k));
        k += 1;
        // A second to settle, then a click's half second: what follows its
        // first 20 ms is the reverb.
        mixer.frames(RATE as usize);
        world.update_audio(clock.at(k));
        k += 1;
        let out = channel(&mixer.frames(RATE as usize / 2), 0);
        let after: f64 = out[RATE as usize / 50..]
            .iter()
            .map(|&s| f64::from(s).powi(2))
            .sum();
        tails.push(after);
    }
    assert!(tails[0] < 1e-9, "the dry room rang: {tails:?}");
    let ratio = tails[2] / tails[1];
    assert!(
        (ratio - 1. / 0.375f64.powi(2)).abs() < 0.7,
        "{ratio:.2} times the energy inside: {tails:?}"
    );
}

#[test]
fn a_zone_left_rings_out() {
    // A click in a warehouse, then the listener steps out into no zone:
    // the warehouse's tail still rings, not cut, as loud as had it stayed.
    let tail = |leave: bool| -> f64 {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        world
            .add_reverb_zone(
                ReverbZone::box_([-10., -5., -10.], [10., 5., 10.])
                    .preset(ReverbPreset::WAREHOUSE)
                    .fade(1.),
            )
            .unwrap();
        let mut click = vec![0f32; RATE as usize];
        click[1] = 0.5;
        let click = Sound::from_samples(&click, 1, RATE).unwrap();
        let beside = spot(&world, [0., 0., -1.]);
        world
            .add_emitter3d(beside, Emitter3d::new(&click).occlusion(false))
            .unwrap();
        let clock = Clock(Instant::now());
        world.set_camera(Camera::looking_at([0., 0., 0.], [0., 0., -1.]));
        world.update_audio(clock.at(0));
        mixer.frames(RATE as usize / 10);
        if leave {
            world.set_camera(Camera::looking_at([40., 0., 0.], [40., 0., -1.]));
        }
        world.update_audio(clock.at(1));
        channel(&mixer.frames(RATE as usize / 2), 0)
            .iter()
            .map(|&s| f64::from(s).powi(2))
            .sum()
    };
    let (stayed, left) = (tail(false), tail(true));
    assert!(stayed > 1e-6, "no tail: {stayed}");
    assert!(
        left > 0.8 * stayed,
        "leaving cut the tail: {left:e} against {stayed:e}"
    );
}

#[test]
fn an_emitter_s_own_mesh_doesn_t_hide_it() {
    let level = |mesh: bool| -> f64 {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let source = if mesh {
            world.add(Object::mesh(
                Geometry::cuboid(1., 1., 1.),
                Material::rgb(0x808080),
            ))
        } else {
            world.add(Object::group())
        };
        world.set_transform(source, Transform::at([0., 0., -6.]));
        world
            .add_emitter3d(source, Emitter3d::new(&tone(4_000.)).looping(true))
            .unwrap();
        world.update_audio(Instant::now());
        level_db(&channel(&mixer.frames(RATE as usize / 4), 0)[RATE as usize / 8..])
    };
    let (alone, boxed) = (level(false), level(true));
    assert!(
        (alone - boxed).abs() < 0.01,
        "{alone:.2} dB, its own box {boxed:.2}"
    );
}

#[test]
fn emitters_that_all_keep_moving_are_rayed_in_turn() {
    // Twenty emitters behind a wall, all moving every frame, four rays a
    // frame: in turn, each is found behind it within five frames, and the
    // low tone they play is the wall's 12 dB down.
    let level = |walled: bool| -> f64 {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        world.set_occlusion_rays(4);
        if walled {
            let wall = world.add(Object::mesh(
                Geometry::cuboid(40., 4., 0.2),
                Material::rgb(0x808080),
            ));
            world.set_transform(wall, Transform::at([0., 0., -3.]));
        }
        let ids: Vec<ObjectId> = (0..20)
            .map(|i| {
                let id = spot(&world, [i as f32 - 10., 0., -6.]);
                world
                    .add_emitter3d(id, Emitter3d::new(&tone(200.)).looping(true).doppler(0.))
                    .unwrap();
                id
            })
            .collect();
        let clock = Clock(Instant::now());
        for k in 0..6 {
            for (i, id) in ids.iter().enumerate() {
                let x = i as f32 - 10. + 0.001 * k as f32;
                world.set_transform(*id, Transform::at([x, 0., -6.]));
            }
            world.update_audio(clock.at(k));
            mixer.frames(FRAME);
        }
        level_db(&channel(&mixer.frames(RATE as usize / 4), 0)[RATE as usize / 8..])
    };
    let lost = level(false) - level(true);
    assert!((lost - 12.04).abs() < 0.5, "{lost:.2} dB behind the wall");
}

#[test]
fn a_dropped_world_s_sounds_stop() {
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    let source = spot(&world, [0., 0., -3.]);
    world
        .add_emitter3d(source, Emitter3d::new(&tone(400.)).looping(true))
        .unwrap();
    world.update_audio(Instant::now());
    mixer.frames(FRAME);
    assert_eq!(audio.stats().voices, 1);
    drop(world);
    // Its fade, then nothing.
    mixer.frames(RATE as usize / 10);
    audio.update();
    assert_eq!(
        audio.stats().voices,
        0,
        "a looping emitter outlived its world"
    );
    assert!(mixer.frames(FRAME).iter().all(|&s| s == 0.));
}

#[test]
fn worlds_share_the_engine_s_world_bus_however_many() {
    // An engine has few buses and none can be taken away: worlds, and a
    // world given its engine again and again, take none of their own.
    let (audio, _mixer) = engine();
    let bus = audio.bus("world").unwrap();
    let worlds: Vec<World> = (0..40)
        .map(|_| {
            let world = World::new();
            world.set_audio(&audio, &bus).unwrap();
            world
        })
        .collect();
    for _ in 0..40 {
        worlds[0].set_audio(&audio, &bus).unwrap();
    }
    // And a bus of the app's own is free for it to make.
    audio.add_bus("mine", &audio.master()).unwrap();
}

#[test]
fn a_one_shot_that_ended_is_let_go() {
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    let blip: Vec<f32> = (0..480).map(|i| (i as f32 * 0.3).sin() * 0.2).collect();
    let blip = Sound::from_samples(&blip, 1, RATE).unwrap();
    let clock = Clock(Instant::now());
    for i in 0..20 {
        let source = spot(&world, [i as f32 - 10., 0., -4.]);
        world.add_emitter3d(source, Emitter3d::new(&blip)).unwrap();
    }
    world.update_audio(clock.at(0));
    // Played out and given back.
    mixer.frames(4 * FRAME);
    audio.update();
    world.update_audio(clock.at(1));
    // The listener walks on: nothing is sent for sounds that are over.
    let sent = audio.stats().commands;
    for k in 2..10 {
        world.set_camera(Camera::looking_at([k as f32, 0., 0.], [k as f32, 0., -1.]));
        world.update_audio(clock.at(k));
        mixer.frames(FRAME);
    }
    assert_eq!(
        audio.stats().commands,
        sent,
        "commands sent for ended sounds"
    );
}

#[test]
fn a_wall_moved_between_them_is_found_among_many_meshes() {
    // 200 meshes high above, a wall off to the side and another under a
    // group, both held still a while (so the rays' tree of boxes is made),
    // then moved, one itself, one by its group, between the listener and
    // a low tone ahead: each time the tone loses the wall's 12 dB.
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    for i in 0..200 {
        let id = world.add(Object::mesh(
            Geometry::cuboid(0.3, 0.3, 0.3),
            Material::rgb(0x808080),
        ));
        world.set_transform(
            id,
            Transform::at([(i % 20) as f32 - 10., 20. + (i / 20) as f32, -4.]),
        );
    }
    let wall = world.add(Object::mesh(
        Geometry::cuboid(4., 4., 0.2),
        Material::rgb(0x808080),
    ));
    world.set_transform(wall, Transform::at([30., 0., -4.]));
    let group = world.add(Object::group());
    world.set_transform(group, Transform::at([-30., 0., 0.]));
    let mut hung = Object::mesh(Geometry::cuboid(4., 4., 0.2), Material::rgb(0x808080));
    hung.parent = Some(group);
    let hung = world.add(hung);
    world.set_transform(hung, Transform::at([0., 0., -5.]));
    let source = spot(&world, [0., 0., -8.]);
    world
        .add_emitter3d(source, Emitter3d::new(&tone(200.)).looping(true).air(false))
        .unwrap();
    let clock = Clock(Instant::now());
    let mut k = 0;
    let mut frames = |world: &World, mixer: &mut Offline, n: usize| {
        for _ in 0..n {
            world.update_audio(clock.at(k));
            k += 1;
            mixer.frames(FRAME);
        }
    };
    let level = |mixer: &mut Offline| {
        level_db(&channel(&mixer.frames(RATE as usize / 4), 0)[RATE as usize / 8..])
    };
    frames(&world, &mut mixer, 5);
    let open = level(&mut mixer);
    world.set_transform(wall, Transform::at([0., 0., -4.]));
    frames(&world, &mut mixer, 2);
    let walled = level(&mut mixer);
    assert!(
        ((open - walled) - 12.04).abs() < 0.5,
        "moved in: {:.2} dB",
        open - walled
    );
    world.set_transform(wall, Transform::at([30., 0., -4.]));
    frames(&world, &mut mixer, 5);
    world.set_transform(group, Transform::at([0., 0., 0.]));
    frames(&world, &mut mixer, 2);
    let hidden = level(&mut mixer);
    assert!(
        ((open - hidden) - 12.04).abs() < 0.5,
        "by its group: {:.2} dB",
        open - hidden
    );
}

#[test]
fn a_frame_s_rays_test_a_bounded_number_of_triangles() {
    // A wall of 125,000 triangles across every ray: past the first, a ray
    // waits for the next frame, so each frame casts one, and all eight
    // emitters are rayed in eight.
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    let n = 250;
    let mut vertices = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            vertices.push(zgx_3d::Vertex {
                position: [
                    i as f32 / n as f32 * 20. - 10.,
                    j as f32 / n as f32 * 4. - 2.,
                    0.,
                ],
                normal: [0., 0., 1.],
                uv: [0., 0.],
            });
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + (n + 1) as u32;
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    let wall = world.add(Object::mesh(
        Geometry::new(vertices, indices).unwrap(),
        Material::rgb(0x808080),
    ));
    world.set_transform(wall, Transform::at([0., 0., -3.]));
    for i in 0..8 {
        let id = spot(&world, [i as f32 - 4., 0., -6.]);
        world
            .add_emitter3d(id, Emitter3d::new(&tone(200.)).looping(true))
            .unwrap();
    }
    let clock = Clock(Instant::now());
    let cast: Vec<usize> = (0..10)
        .map(|k| {
            world.update_audio(clock.at(k));
            mixer.frames(FRAME);
            world.audio_rays()
        })
        .collect();
    assert_eq!(cast, [1, 1, 1, 1, 1, 1, 1, 1, 0, 0]);
}

#[test]
fn a_world_shown_twice_a_frame_sounds_as_one_shown_once() {
    // Two boxes (or two renderers) each run the world's sound after their
    // frame, at the same frame time: the second must change nothing, or a
    // moving sound's Doppler and pan flip each frame.
    let run = |shown: usize| -> (u64, Vec<f32>) {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let at = |k: usize| [-20. + 40. * k as f32 * 0.01, 0., -4.];
        let car = spot(&world, at(0));
        world
            .add_emitter3d(car, Emitter3d::new(&tone(400.)).looping(true))
            .unwrap();
        let clock = Clock(Instant::now());
        let mut out = Vec::new();
        for k in 0..60 {
            world.set_transform(car, Transform::at(at(k)));
            for _ in 0..shown {
                world.update_audio(clock.at(k));
            }
            out.extend(mixer.frames(FRAME));
        }
        (audio.stats().commands, out)
    };
    let (once, twice) = (run(1), run(2));
    assert_eq!(once.0, twice.0, "commands sent");
    assert!(once.1 == twice.1, "what was heard differs");
}

#[test]
fn a_world_s_view_shown_in_two_boxes_steps_its_sound_once_a_frame() {
    // Through a real renderer, the times the frames' own: the world's view
    // mounted in two boxes sends what it sends mounted in one, a frame at a
    // time, as a sound moves across it.
    use zgx::{compose::prelude::*, scene::Color, widgets::Ui};
    let commands = |boxes: usize| -> Option<u64> {
        let mut gpu = match zgx_gpu::GpuRenderer::new(100, 100) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let car = spot(&world, [-20., 0., -4.]);
        world
            .add_emitter3d(car, Emitter3d::new(&tone(400.)).looping(true))
            .unwrap();
        let mut ui = Ui::new(100., 100.);
        let mut root = div().size(100., 100.).bg(Color(0, 0, 0, 255));
        for k in 0..boxes {
            root = root.child(
                gpu_view("World", world.view())
                    .absolute()
                    .left(50. * k as f32)
                    .size(50., 100.),
            );
        }
        ui.mount(root);
        let sent = audio.stats().commands;
        for k in 0..30 {
            world.set_transform(car, Transform::at([-20. + k as f32, 0., -4.]));
            ui.prepare_frame();
            let report = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
            mixer.frames(FRAME);
        }
        Some(audio.stats().commands - sent)
    };
    let (Some(once), Some(twice)) = (commands(1), commands(2)) else {
        return;
    };
    assert!(once > 0, "nothing was sent: the view never drew");
    assert_eq!(once, twice, "commands sent, one box against two");
}

#[cfg(feature = "hrtf")]
#[test]
fn a_binaural_emitter_beside_the_listener_is_heard_through_a_head() {
    // Noise 3 m to the right, then to the left. Panned, the far ear would
    // hear nothing; through the head it hears it quieter (the head's
    // shadow) and later (the way round it).
    // Added at `from`, then moved to `x`: the head follows it.
    let heard = |from: f32, x: f32, binaural: bool| -> (Vec<f32>, Vec<f32>) {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<f32> = (0..RATE as usize)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
            })
            .collect();
        let noise = Sound::from_samples(&noise, 1, RATE).unwrap();
        let at = spot(&world, [from, 0., 0.]);
        let emitter = Emitter3d::new(&noise)
            .looping(true)
            .occlusion(false)
            .air(false)
            .binaural(binaural);
        world.add_emitter3d(at, emitter).unwrap();
        let clock = Clock(Instant::now());
        world.update_audio(clock.at(0));
        world.set_transform(at, Transform::at([x, 0., 0.]));
        world.update_audio(clock.at(1));
        mixer.frames(RATE as usize / 10);
        let out = mixer.frames(RATE as usize / 2);
        (channel(&out, 0), channel(&out, 1))
    };
    let energy = |s: &[f32]| s.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>();
    // The lag (in frames) at which `late` best matches `early`.
    let lag = |early: &[f32], late: &[f32]| -> i32 {
        (-60i32..=60)
            .max_by(|&a, &b| {
                let at = |k: i32| -> f64 {
                    (100..early.len() - 100)
                        .map(|i| f64::from(early[i]) * f64::from(late[(i as i32 + k) as usize]))
                        .sum()
                };
                at(a).total_cmp(&at(b))
            })
            .unwrap()
    };
    let (left, right) = heard(3., 3., false);
    assert!(
        energy(&left) < 1e-9 * energy(&right),
        "panned, the left hears it"
    );
    let panned = energy(&right);
    for (x, near, far) in [(3f32, 1, 0), (-3., 0, 1)] {
        let (left, right) = heard(-x, x, true);
        let ears = [left, right];
        let db = 10. * (energy(&ears[far]) / energy(&ears[near])).log10();
        assert!(
            (-25.0..-2.0).contains(&db),
            "at {x} m the far ear is {db:.1} dB"
        );
        // A head 8.75 cm across: about 0.65 ms round, 31 frames.
        let behind = lag(&ears[near], &ears[far]);
        // Centred into the head at unity: the spherical head lifts the
        // near ear's highs, so white noise comes out 5.3 dB over the
        // panned (its near ear alone).
        let over = 10. * ((energy(&ears[near]) + energy(&ears[far])) / panned).log10();
        assert!(
            (3.3..7.3).contains(&over),
            "at {x} m it's {over:.1} dB over the panned"
        );
        assert!(
            (15..=45).contains(&behind),
            "at {x} m the far ear is {behind} frames late"
        );
    }
}

#[test]
fn an_emitter_s_own_pitch_multiplies_doppler_s() {
    // A 400 Hz loop at pitch 1.5: still, 600 Hz; coming at 40 m/s, 600
    // times Doppler's rise.
    let heard = |moving: bool| -> (f64, f64) {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let at = |k: usize| {
            let x = if moving {
                -40. + 40. * k as f32 * 0.01
            } else {
                -20.
            };
            [x, 0., -4.]
        };
        let car = spot(&world, at(0));
        world
            .add_emitter3d(
                car,
                Emitter3d::new(&tone(400.))
                    .looping(true)
                    .pitch(1.5)
                    .falloff(Falloff::inverse(2., 100.))
                    .air(false)
                    .occlusion(false),
            )
            .unwrap();
        let clock = Clock(Instant::now());
        world.update_audio(clock.at(0));
        let mut out = Vec::new();
        for k in 1..=30 {
            world.set_transform(car, Transform::at(at(k)));
            world.update_audio(clock.at(k));
            out.extend(mixer.frames(FRAME));
        }
        let still = [0.; 3];
        let velocity = if moving { [40., 0., 0.] } else { still };
        let want = 600. * f64::from(doppler(at(23), velocity, still, still, 1.));
        (frequency(&channel(&out, 0)[20 * FRAME..25 * FRAME]), want)
    };
    for moving in [false, true] {
        let (got, want) = heard(moving);
        assert!(
            ((got - want) / want).abs() < 0.005,
            "moving {moving}: {got:.2} Hz, not {want:.2}"
        );
    }
    assert!(heard(true).0 > 640., "Doppler's rise on top");
}

#[test]
fn an_emitter_started_partway_starts_there() {
    // Half a second of silence, then a tone: started half a second in, it
    // sounds at once; from its start, not for half a second.
    let mut samples = vec![0f32; RATE as usize / 2];
    samples.extend(
        (0..RATE as usize / 2)
            .map(|i| (i as f32 * 440. * std::f32::consts::TAU / RATE as f32).sin() * 0.5),
    );
    let sound = Sound::from_samples(&samples, 1, RATE).unwrap();
    let first = |start: f32| -> f64 {
        let (audio, mut mixer) = engine();
        let world = world(&audio);
        let here = spot(&world, [0., 0., -2.]);
        world
            .add_emitter3d(here, Emitter3d::new(&sound).start(start).occlusion(false))
            .unwrap();
        world.update_audio(Clock(Instant::now()).at(0));
        level_db(&channel(&mixer.frames(FRAME), 0))
    };
    assert!(first(0.) < -120., "from its start: {:.1} dB", first(0.));
    assert!(first(0.5) > -30., "half a second in: {:.1} dB", first(0.5));
    assert!(first(1.75) < -120., "a one-shot past its end is silent");
}

#[cfg(feature = "hrtf")]
#[test]
fn a_binaural_source_stepping_across_the_back_stays_behind() {
    // Noise 3 m behind, 1 degree to the right, steps to 1 degree to the
    // left: its head turns from 179 to -179 through 180, the short way.
    // Through the ramp, block by block (the head's), the ears stay within
    // 3 dB of each other; the long way round, through hard right and hard
    // left, they'd part by about 13.
    let (audio, mut mixer) = engine();
    let world = world(&audio);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let noise: Vec<f32> = (0..RATE as usize)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    let noise = Sound::from_samples(&noise, 1, RATE).unwrap();
    let side = 3. * 1f32.to_radians().tan();
    let at = spot(&world, [side, 0., 3.]);
    world
        .add_emitter3d(
            at,
            Emitter3d::new(&noise)
                .looping(true)
                .occlusion(false)
                .air(false)
                .binaural(true),
        )
        .unwrap();
    let clock = Clock(Instant::now());
    world.update_audio(clock.at(0));
    mixer.frames(RATE as usize / 5);
    world.set_transform(at, Transform::at([-side, 0., 3.]));
    world.update_audio(clock.at(1));
    let out = mixer.frames(RATE as usize / 10);
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    let energy = |s: &[f32]| s.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>();
    let widest = left
        .chunks(128)
        .zip(right.chunks(128))
        .map(|(l, r)| (10. * (energy(l) / energy(r)).log10()).abs())
        .fold(0f64, f64::max);
    assert!(widest < 3., "the ears parted by {widest:.1} dB");
}
