//! What the demo's harbor costs in sound, measured: 2,000 containers in
//! stacks between the listener and its sounds; two crane motors moving, a
//! generator, a buoy bell bobbing, three gulls circling and a horn far
//! off, each an emitter with occlusion and the air; a gantry (a warehouse)
//! and a cabin (a room) as reverb zones; two rain layers streamed; the
//! wind a noise loop through a moving low-pass. The listener walks through
//! both zones at 60 frames a second. Each frame: the world's sound on the
//! main thread (`World::update_audio`), then its 800 frames mixed, offline.
//! All its sounds are made here.
//!
//! `cargo run --release -p zgx-3d --features audio --example audio_scene`
use std::time::{Duration, Instant};
use zgx_3d::{
    Camera, Emitter3d, Geometry, Material, Object, ReverbPreset, ReverbZone, Transform, World,
};
use zgx_audio::effects::LowPass;
use zgx_audio::spatial::Falloff;
use zgx_audio::{Audio, Config, Play, Ramp, Sound, Source, wav};

const RATE: u32 = 48_000;
const FRAME: usize = 800;

/// A second of something, made from `f(i)`, looped seamlessly by being a
/// whole number of its cycles.
fn sound(channels: usize, f: impl FnMut(usize) -> f32) -> Sound {
    let samples: Vec<f32> = (0..RATE as usize * channels).map(f).collect();
    Sound::from_samples(&samples, channels, RATE).unwrap()
}

fn noise(seed: u64) -> impl FnMut() -> f32 {
    let mut s = seed;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }
}

fn main() {
    let (audio, mut mixer) = Audio::offline(Config::default()).unwrap();
    let world = World::new();
    world
        .set_audio(&audio, &audio.bus("world").unwrap())
        .unwrap();
    // The containers: 2,000 in stacks of 4 along the quay.
    for i in 0..2_000 {
        let id = world.add(Object::mesh(
            Geometry::cuboid(6., 2.6, 2.4),
            Material::rgb(0x8a3b2a),
        ));
        let (row, stack) = (i / 4, i % 4);
        let (x, z) = ((row % 50) as f32 * 7. - 175., (row / 50) as f32 * 9. - 60.);
        world.set_transform(id, Transform::at([x, 1.3 + stack as f32 * 2.6, z]));
    }
    // The sounds, made here.
    let tau = std::f32::consts::TAU;
    let motor = sound(1, |i| (i as f32 * 60. * tau / RATE as f32).sin() * 0.3);
    let mut hum = noise(1);
    let generator = sound(1, |i| {
        (i as f32 * 50. * tau / RATE as f32).sin() * 0.2 + hum() * 0.02
    });
    let bell = sound(1, |i| {
        let t = i as f32 / RATE as f32;
        (t * 880. * tau).sin() * (-t * 3.).exp() * 0.4
    });
    let gull = sound(1, |i| (i as f32 * 1_500. * tau / RATE as f32).sin() * 0.1);
    let horn = sound(1, |i| (i as f32 * 110. * tau / RATE as f32).sin() * 0.4);
    let place = |at: [f32; 3]| {
        let id = world.add(Object::group());
        world.set_transform(id, Transform::at(at));
        id
    };
    let cranes = [place([-40., 20., -30.]), place([30., 20., -30.])];
    let buoy = place([0., 0., -120.]);
    let gulls = [
        place([0., 30., 0.]),
        place([20., 25., 10.]),
        place([-30., 35., -20.]),
    ];
    let emit = |id, s: &Sound, falloff: Falloff| {
        world
            .add_emitter3d(id, Emitter3d::new(s).looping(true).falloff(falloff))
            .unwrap();
    };
    for c in cranes {
        emit(c, &motor, Falloff::inverse(5., 150.));
    }
    emit(place([10., 1., 20.]), &generator, Falloff::inverse(3., 80.));
    emit(buoy, &bell, Falloff::inverse(5., 300.));
    for g in gulls {
        emit(g, &gull, Falloff::inverse(4., 120.));
    }
    emit(
        place([200., 5., -400.]),
        &horn,
        Falloff::inverse(20., 1_000.),
    );
    world
        .add_reverb_zone(
            ReverbZone::box_([-60., 0., -40.], [60., 30., -10.])
                .preset(ReverbPreset::WAREHOUSE)
                .fade(8.),
        )
        .unwrap();
    world
        .add_reverb_zone(
            ReverbZone::box_([5., 0., 15.], [15., 4., 25.])
                .preset(ReverbPreset::ROOM)
                .fade(2.),
        )
        .unwrap();
    // Rain: two layers, streamed, looped.
    let mut rain = noise(7);
    let layer: Vec<f32> = (0..RATE as usize * 2 * 4).map(|_| rain() * 0.05).collect();
    let layer = wav::write(&layer, 2, RATE, wav::Encoding::Float32).unwrap();
    // (`-- no-streams` leaves them out: an offline render waits for the
    // decode thread to fill them first, which a device's callback never
    // does, so without them what's timed is the mixing alone.)
    let streams = !std::env::args().any(|a| a == "no-streams");
    for _ in 0..if streams { 2 } else { 0 } {
        audio
            .stream(Source::bytes(layer.clone()), Play::new().looping(true))
            .unwrap();
    }
    // Wind: noise through a low-pass that moves.
    let wind_bus = audio.add_bus("wind", &audio.master()).unwrap();
    let gust = wind_bus.add_effect(LowPass::new(600.)).unwrap();
    let mut breath = noise(9);
    let wind = sound(1, |_| breath() * 0.2);
    audio
        .play(&wind, Play::new().bus(&wind_bus).looping(true))
        .unwrap();
    // A minute's walk through the gantry and into the cabin and out.
    let start = Instant::now();
    let (mut world_time, mut mix_time) = (Vec::new(), Vec::new());
    for k in 0..3_600u32 {
        let t = k as f32 / 60.;
        let x = -50. + t * 1.2;
        let z = if t < 30. {
            -25.
        } else {
            -25. + (t - 30.) * 1.5
        };
        world.set_camera(Camera::looking_at([x, 1.7, z], [x + 1., 1.7, z]));
        for (n, c) in cranes.iter().enumerate() {
            let swing = (t * 0.3 + n as f32).sin() * 15.;
            world.set_transform(
                *c,
                Transform::at([-40. + 70. * n as f32 + swing, 20., -30.]),
            );
        }
        world.set_transform(buoy, Transform::at([0., (t * 1.3).sin() * 0.5, -120.]));
        for (n, g) in gulls.iter().enumerate() {
            let a = t * 0.5 + n as f32 * 2.;
            world.set_transform(
                *g,
                Transform::at([a.cos() * 40., 25. + n as f32 * 5., a.sin() * 40.]),
            );
        }
        if k % 30 == 0 {
            let _ = gust.set(
                LowPass::CUTOFF,
                400. + (t * 0.7).sin().abs() * 1_500.,
                Ramp::ms(500.),
            );
        }
        let began = Instant::now();
        world.update_audio(start + Duration::from_secs_f32(t));
        world_time.push(began.elapsed());
        let began = Instant::now();
        mixer.frames(FRAME);
        mix_time.push(began.elapsed());
    }
    let report = |name: &str, times: &mut Vec<Duration>| {
        times.sort();
        let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        println!(
            "{name:<22} median {:>8.1?}  99th {:>8.1?}  most {:>8.1?}",
            at(0.5),
            at(0.99),
            times[times.len() - 1]
        );
    };
    println!("a frame (60 a second, 800 frames of sound each):");
    report("world's sound (main)", &mut world_time);
    report("mixing its 800 frames", &mut mix_time);
    let period = Duration::from_secs_f64(FRAME as f64 / f64::from(RATE));
    println!(
        "mixing, of real time: median {:.2}%, 99th {:.2}%",
        100. * mix_time[mix_time.len() / 2].as_secs_f64() / period.as_secs_f64(),
        100. * mix_time[mix_time.len() * 99 / 100].as_secs_f64() / period.as_secs_f64()
    );
    let stats = audio.stats();
    println!("voices {}, streams {}", stats.voices, stats.streams);
}
