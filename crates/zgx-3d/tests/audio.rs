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
    // The same reverb either side, only the left's unheard: halfway, it's
    // heard at half its wet (each zone half heard), so a sixteenth of the
    // energy.
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
        (ratio - 16.).abs() < 1.6,
        "{ratio:.2} times the energy inside: {tails:?}"
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
