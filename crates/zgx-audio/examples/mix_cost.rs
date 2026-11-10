//! What a voice costs the mixer, in instructions retired on this thread
//! (macOS; elsewhere, nanoseconds) for each second of sound it mixes.
//!
//! `cargo run --release -p zgx-audio --example mix_cost -- [voices] [source rate] [channels] [block]`
//!
//! Each arm renders 10 seconds offline in blocks of `block` frames (512 by
//! default), the voices looping a noise sound spread over three buses with
//! the limiter on, and prints the least of 5 interleaved runs. The control
//! arm, no voices, is subtracted. Each arm checks it mixed what it says:
//! every voice playing and sound in the output.
use zgx_audio::{Audio, Config, Play, Sound};

const SECONDS: usize = 10;

#[cfg(target_os = "macos")]
fn count() -> u64 {
    unsafe extern "C" {
        fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32;
    }
    let mut counts = [0u64; 2];
    // SAFETY: kind 1 (THSC_CPI) fills two u64s: instructions, then cycles.
    let ok = unsafe { thread_selfcounts(1, counts.as_mut_ptr(), 16) } == 0;
    assert!(ok, "this kernel doesn't count a thread's instructions");
    counts[0]
}

#[cfg(not(target_os = "macos"))]
fn count() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

/// Instructions (and seconds) to mix `voices` voices of `sound` for
/// [`SECONDS`].
fn run(voices: usize, sound: &Sound, block: usize) -> (u64, f64) {
    let (audio, mut mixer) = Audio::offline(Config {
        voices: voices.max(1),
        ..Config::default()
    });
    let buses = ["ui", "music", "world"].map(|name| audio.bus(name).unwrap());
    for i in 0..voices {
        let play = Play::new()
            .bus(&buses[i % 3])
            .gain(0.5 / voices as f32)
            .pan((i as f32 / voices as f32) * 2. - 1.)
            .looping(true);
        audio.play(sound, play).unwrap();
    }
    let mut out = vec![0f32; block * 2];
    // The first block applies the plays; it isn't counted.
    mixer.render(&mut out);
    let mut energy = 0f64;
    let started = std::time::Instant::now();
    let before = count();
    for _ in 0..SECONDS * 48_000 / block {
        mixer.render(&mut out);
        energy += f64::from(out[0].abs());
    }
    let instructions = count() - before;
    let seconds = started.elapsed().as_secs_f64();
    assert_eq!(
        audio.stats().voices,
        voices as u64,
        "every voice still plays"
    );
    assert!(voices == 0 || energy > 0., "sound came out");
    std::hint::black_box(&out);
    (instructions, seconds)
}

fn main() {
    let mut args = std::env::args()
        .skip(1)
        .map(|a| a.parse::<usize>().expect("numbers"));
    let voices = args.next().unwrap_or(64);
    let rate = args.next().unwrap_or(48_000) as u32;
    let channels = args.next().unwrap_or(2);
    let block = args.next().unwrap_or(512);
    // A second of noise, so nothing in the mix is cheaper for being regular.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let noise: Vec<f32> = (0..rate as usize * channels)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    let sound = Sound::from_samples(&noise, channels, rate).unwrap();
    let (mut control, mut arm) = ((u64::MAX, f64::MAX), (u64::MAX, f64::MAX));
    for _ in 0..5 {
        let c = run(0, &sound, block);
        let a = run(voices, &sound, block);
        control = (control.0.min(c.0), control.1.min(c.1));
        arm = (arm.0.min(a.0), arm.1.min(a.1));
    }
    let per_voice = (arm.0 - control.0) as f64 / voices.max(1) as f64 / SECONDS as f64;
    let realtime = arm.1 / SECONDS as f64 * 100.;
    let unit = if cfg!(target_os = "macos") {
        "instructions"
    } else {
        "ns"
    };
    println!(
        "{voices} voices, {channels} ch at {rate} Hz, blocks of {block}: {:.0} {unit} a voice a second \
         ({:.0} per frame); control {:.2} M {unit}/s; all voices {:.3}% of real time",
        per_voice,
        per_voice / 48_000.,
        control.0 as f64 / SECONDS as f64 / 1e6,
        realtime
    );
}
