//! zgx-audio's arm of the comparison (miniaudio's is miniaudio/bench.c):
//! 64 voices each looping its own second of stereo noise over three buses,
//! rendered offline, and its output compared with miniaudio's.
//! Each run counts the instructions its render loop retires on this thread
//! (macOS: `thread_selfcounts`). See README.md; `run.py` runs everything.
//!
//!   audio-compare [voices] [rate] [block] [pan: spread|center] [ramps: none|gain|gainpan|all]
//!                 [limiter: on|off] [interpolation: cubic|linear] [layout: buses|master|silent]
//!   audio-compare sound [dir]   every engine's 10 s kept and compared (miniaudio's from `dir`)
const SECONDS: usize = 10;

fn count() -> u64 {
    unsafe extern "C" {
        fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32;
    }
    let mut counts = [0u64; 2];
    assert_eq!(unsafe { thread_selfcounts(1, counts.as_mut_ptr(), 16) }, 0);
    counts[0]
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ramps {
    None,
    Gain,
    GainPan,
    All,
}

#[derive(Clone, Copy, Debug)]
struct Scene {
    voices: usize,
    rate: u32,
    block: usize,
    spread: bool,
    ramps: Ramps,
    limiter: bool,
    cubic: bool,
    /// Where zgx-audio's voices go: "buses", "master", or "silent" (gain 0).
    layout: &'static str,
}

impl Scene {
    fn gain(&self) -> f32 {
        0.5 / self.voices.max(1) as f32
    }
    fn pan(&self, i: usize) -> f32 {
        if self.spread {
            (i as f32 / self.voices as f32) * 2. - 1.
        } else {
            0.
        }
    }
    fn blocks(&self) -> usize {
        SECONDS * 48_000 / self.block
    }
}

/// A second of stereo noise, one sound per voice (64 of them), so no two
/// voices read the same memory or sum to a louder copy of one.
fn noises(rate: u32) -> Vec<Vec<f32>> {
    (0..64).map(|v| noise(rate, v)).collect()
}

fn noise(rate: u32, voice: u64) -> Vec<f32> {
    let mut seed = 0x2545_f491_4f6c_dd1du64 ^ (voice.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    (0..rate as usize * 2)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect()
}

/// Instructions for the scene's blocks after the first, and the output if
/// asked for.
fn zgx(scene: Scene, noises: &[Vec<f32>], keep: bool) -> (u64, Vec<f32>) {
    use zgx_audio::{Audio, Config, Play, Ramp, Sound};
    let interpolation = if scene.cubic {
        zgx_audio::Interpolation::Cubic
    } else {
        zgx_audio::Interpolation::Linear
    };
    let config = Config {
        voices: scene.voices.max(1),
        limiter: scene.limiter.then_some(1.),
        interpolation,
        ..Config::default()
    };
    let (audio, mut mixer) = Audio::offline(config).expect("a config the engine runs with");
    let sounds: Vec<Sound> = noises
        .iter()
        .map(|n| Sound::from_samples(n, 2, scene.rate).unwrap())
        .collect();
    let buses = ["ui", "music", "world"].map(|name| audio.bus(name).unwrap());
    let mut voices = Vec::new();
    for i in 0..scene.voices {
        let gain = if scene.layout == "silent" {
            0.
        } else {
            scene.gain()
        };
        let mut play = Play::new().gain(gain).pan(scene.pan(i)).looping(true);
        if scene.layout != "master" {
            play = play.bus(&buses[i % 3]);
        }
        voices.push(audio.play(&sounds[i % sounds.len()], play).unwrap());
    }
    let mut out = vec![0f32; scene.block * 2];
    let mut kept = Vec::new();
    mixer.render(&mut out);
    if keep {
        kept.extend_from_slice(&out);
    }
    let long = Ramp::seconds(SECONDS as f64 + 1.);
    for (i, voice) in voices.iter().enumerate() {
        if scene.ramps != Ramps::None {
            voice.set_gain(scene.gain() * 0.5, long).unwrap();
        }
        if matches!(scene.ramps, Ramps::GainPan | Ramps::All) {
            voice.set_pan(-scene.pan(i) * 0.5 + 0.25, long).unwrap();
        }
        if scene.ramps == Ramps::All {
            voice.set_pitch(1.02, long).unwrap();
        }
    }
    let before = count();
    for _ in 1..scene.blocks() {
        mixer.render(&mut out);
        if keep {
            kept.extend_from_slice(&out);
        }
    }
    let instructions = count() - before;
    std::hint::black_box(&out);
    assert_eq!(
        audio.stats().voices,
        scene.voices as u64,
        "every voice still plays"
    );
    (instructions, kept)
}

fn cost(scene: Scene) {
    let noise = noises(scene.rate);
    let control = Scene { voices: 0, ..scene };
    let mut best = [u64::MAX; 2];
    for _ in 0..5 {
        best[0] = best[0].min(zgx(control, &noise, false).0);
        best[1] = best[1].min(zgx(scene, &noise, false).0);
    }
    let frames = ((scene.blocks() - 1) * scene.block) as f64;
    let per = |arm: u64, control: u64| (arm - control) as f64 / scene.voices as f64 / frames;
    println!(
        "{scene:?}
  zgx-audio {:.2} a voice a frame (control {:.3} a frame)",
        per(best[1], best[0]),
        best[0] as f64 / frames,
    );
}

struct Measure {
    rms: [f64; 2],
    peak: f32,
}

fn measure(samples: &[f32]) -> Measure {
    let mut sum = [0f64; 2];
    for frame in samples.chunks_exact(2) {
        sum[0] += f64::from(frame[0]).powi(2);
        sum[1] += f64::from(frame[1]).powi(2);
    }
    let n = (samples.len() / 2) as f64;
    Measure {
        rms: [(sum[0] / n).sqrt(), (sum[1] / n).sqrt()],
        peak: samples.iter().fold(0f32, |p, s| p.max(s.abs())),
    }
}

/// Correlation of one channel of `a` and `b` (interleaved stereo), `b`
/// shifted by `lag` frames, and their largest difference.
fn correlate(a: &[f32], b: &[f32], lag: isize, channel: usize) -> (f64, f32) {
    let frames = a.len().min(b.len()) / 2;
    let (mut ab, mut aa, mut bb, mut worst) = (0f64, 0f64, 0f64, 0f32);
    for i in 960..frames - 960 {
        let j = (i as isize + lag) as usize;
        let (x, y) = (a[i * 2 + channel], b[j * 2 + channel]);
        ab += f64::from(x) * f64::from(y);
        aa += f64::from(x).powi(2);
        bb += f64::from(y).powi(2);
        worst = worst.max((x - y).abs());
    }
    (ab / (aa * bb).sqrt(), worst)
}

fn report(name: &str, reference: &[f32], other: &[f32]) {
    let m = measure(other);
    let best = (-4..=4)
        .map(|lag| (lag, correlate(reference, other, lag, 0)))
        .max_by(|a, b| a.1.0.partial_cmp(&b.1.0).unwrap())
        .unwrap();
    let right = correlate(reference, other, best.0, 1);
    println!(
        "  {name:10} rms L {:.6} R {:.6}, peak {:.4}; against zgx-audio, {} frames later: correlation L {:.7} R {:.7}, largest difference {:.1e} / {:.1e}",
        m.rms[0], m.rms[1], m.peak, best.0, best.1.0, right.0, best.1.1, right.1
    );
}

fn sound(rate: u32, spread: bool, cubic: bool, miniaudio: Option<&str>) {
    let scene = Scene {
        voices: 64,
        rate,
        block: 512,
        spread,
        ramps: Ramps::None,
        limiter: false,
        cubic,
        layout: "buses",
    };
    let noise = noises(rate);
    let (_, ours) = zgx(scene, &noise, true);
    println!("{scene:?}");
    let m = measure(&ours);
    println!(
        "  zgx-audio  rms L {:.6} R {:.6}, peak {:.4}",
        m.rms[0], m.rms[1], m.peak
    );
    let name = format!("{rate}-{}", if spread { "spread" } else { "center" });
    if let Some(Ok(bytes)) = miniaudio.map(|dir| std::fs::read(format!("{dir}/out-{name}.f32"))) {
        let theirs: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        report("miniaudio", &ours, &theirs);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("sound") {
        for rate in [48_000, 44_100] {
            for spread in [false, true] {
                for cubic in [true, false] {
                    if cubic || rate != 48_000 {
                        sound(rate, spread, cubic, args.get(1).map(String::as_str));
                    }
                }
            }
        }
        return;
    }
    let arg = |i: usize, default: &str| args.get(i).cloned().unwrap_or_else(|| default.into());
    cost(Scene {
        voices: arg(0, "64").parse().unwrap(),
        rate: arg(1, "48000").parse().unwrap(),
        block: arg(2, "512").parse().unwrap(),
        spread: arg(3, "spread") == "spread",
        ramps: match arg(4, "none").as_str() {
            "gain" => Ramps::Gain,
            "gainpan" => Ramps::GainPan,
            "all" => Ramps::All,
            _ => Ramps::None,
        },
        limiter: arg(5, "on") == "on",
        cubic: arg(6, "cubic") == "cubic",
        layout: match arg(7, "buses").as_str() {
            "master" => "master",
            "silent" => "silent",
            _ => "buses",
        },
    });
}
