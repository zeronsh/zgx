//! Twenty seconds of the harbor's sound, mixed offline and written to a
//! WAV, for listening: the scene built, the camera walked along the
//! bench's path at 60 frames a second, the world's sound stepped and 800
//! frames mixed each frame. Nothing reaches a speaker: the engine is
//! offline (no device is opened), and no GPU or window is made.
//!
//! Run (from examples/harbor):
//! `cargo run --release --bin harbor_listen -- out.wav [start seconds]`
//!
//! It prints master's RMS and peak, each second's RMS, the longest quiet
//! stretch, and each bus's level as `Bus::level` metered it. The WAV is
//! 48 kHz, 16-bit, stereo; it isn't for the repository.
//!
//! Without a renderer the cranes stand still: their animators are stepped
//! by the frames a view draws, and nothing public steps them otherwise
//! (`World::animate` is hidden), so their drives sound from where they
//! stand.
use std::time::{Duration, Instant};
use zgx_3d::World;
use zgx_audio::{Level, wav};
use zgx_harbor::{
    path,
    scene::{Config, Harbor},
    sound::HarborSound,
    sounds::RATE,
};

const SECONDS: usize = 20;
const FPS: usize = 60;

fn db(x: f64) -> f64 {
    20. * x.max(1e-12).log10()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .expect("usage: harbor_listen <out.wav> [start seconds]");
    let start: f32 = args
        .next()
        .map_or(0., |s| s.parse().expect("a start in seconds"));
    let config = Config::default();
    let world = World::new();
    // No sky: sound doesn't see it.
    let harbor = Harbor::build(&world, &config, None);
    let mut sound = HarborSound::offline(&world, &harbor).expect("the harbor's sound");
    let names = ["master", "world", "ui", "music"];
    let buses: Vec<_> = names
        .iter()
        .map(|&name| {
            let bus = if name == "master" {
                sound.audio.master()
            } else {
                sound.audio.bus(name).expect("a stock bus")
            };
            bus.set_metered(true).expect("metered");
            bus
        })
        .collect();
    // Each bus's loudest, and its RMS's mean square over the frames.
    let mut levels = vec![(0f32, 0f64); buses.len()];
    let base = world.camera();
    let began = Instant::now();
    let step = 1. / FPS as f32;
    let mut samples = Vec::with_capacity(SECONDS * RATE as usize * 2);
    for k in 0..SECONDS * FPS {
        let t = start + k as f32 * step;
        let camera = path::camera(t, base);
        world.set_camera(camera);
        if let Some(rain) = harbor.rain {
            rain.follow(&world, camera.position);
        }
        world.update_audio(began + Duration::from_secs_f32(k as f32 * step));
        samples.extend(sound.mix(step));
        for (bus, (peak, square)) in buses.iter().zip(&mut levels) {
            let Level { rms, peak: p } = bus.level();
            *peak = peak.max(p);
            *square += f64::from(rms) * f64::from(rms);
        }
    }
    let frames = samples.len() / 2;
    let mixed = began.elapsed();
    // Master, from its samples: RMS, peak, clipped samples.
    let square: f64 = samples.iter().map(|&x| f64::from(x).powi(2)).sum();
    let rms = (square / samples.len() as f64).sqrt();
    let peak = samples.iter().fold(0f32, |m, &x| m.max(x.abs()));
    let clipped = samples.iter().filter(|x| x.abs() >= 1.).count();
    println!(
        "{frames} frames ({:.1} s) from {start} s along the path, mixed in {mixed:.1?}",
        frames as f64 / f64::from(RATE)
    );
    println!(
        "master: {:.1} dBFS RMS, {:.1} dBFS peak, {clipped} samples at full scale",
        db(rms),
        db(f64::from(peak))
    );
    // Each second's RMS, and the longest stretch of 10 ms windows under
    // -60 dBFS.
    let second = RATE as usize * 2;
    let each: Vec<String> = samples
        .chunks(second)
        .map(|s| {
            let e: f64 = s.iter().map(|&x| f64::from(x).powi(2)).sum();
            format!("{:.0}", db((e / s.len() as f64).sqrt()))
        })
        .collect();
    println!("each second, dBFS RMS: {}", each.join(" "));
    let window = RATE as usize / 100 * 2;
    let (mut run, mut longest) = (0usize, 0usize);
    for s in samples.chunks(window) {
        let e: f64 = s.iter().map(|&x| f64::from(x).powi(2)).sum();
        if db((e / s.len() as f64).sqrt()) < -60. {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    println!("longest stretch under -60 dBFS: {} ms", longest * 10);
    for (name, (peak, square)) in names.iter().zip(&levels) {
        let rms = (square / (SECONDS * FPS) as f64).sqrt();
        println!(
            "bus {name:<6}: {:.1} dBFS RMS (metered), {:.1} dBFS peak",
            db(rms),
            db(f64::from(*peak))
        );
    }
    let stats = sound.audio.stats();
    println!(
        "{} emitters, voices {}, underruns {}, ran dry {}",
        sound.emitters, stats.voices, stats.underruns, stats.dry
    );
    let bytes = wav::write(&samples, 2, RATE, wav::Encoding::Pcm16).expect("a WAV");
    std::fs::write(&out, bytes).expect("the WAV written");
    println!("wrote {out}");
}
