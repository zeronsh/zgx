//! zgx-audio's side of the effect comparison: one instance of a stock
//! effect on 10 s of the noise miniaudio/effects.c runs its own through,
//! compared with what miniaudio wrote. Past the first 0.1 s (a zgx-audio
//! voice starting away from zero fades in over 1 ms) the two must agree to
//! 90 dB under the signal, where the algorithms are the same.
//!
//!   effects <lowpass|lowpass24|highpass|peaking> <miniaudio's output.f32>
use zgx_audio::effects::{HighPass, LowPass, Peaking, Slope};
use zgx_audio::{Audio, Config, Play, Sound};

/// The first of effects.c's sounds: a second of stereo noise.
fn noise() -> Vec<f32> {
    let mut seed = 0x2545_f491_4f6c_dd1du64 ^ 1;
    (0..48_000 * 2)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (name, theirs) = match &args[..] {
        [name, path] => (name.as_str(), path),
        _ => panic!("effects <lowpass|lowpass24|highpass|peaking> <miniaudio's output.f32>"),
    };
    let (audio, mut mixer) = Audio::offline(Config {
        limiter: None,
        ..Config::default()
    })
    .expect("a config the engine runs with");
    let master = audio.master();
    let added = match name {
        // As effects.c makes miniaudio's: the same cutoffs, Qs and gains.
        "lowpass" => master.add_effect(LowPass::new(2000.)),
        "lowpass24" => master.add_effect(LowPass::new(2000.).slope(Slope::Db24)),
        "highpass" => master.add_effect(HighPass::new(200.)),
        "peaking" => master.add_effect(Peaking::new(1000., 6., 1.)),
        other => panic!("no effect named {other:?}"),
    };
    added.expect("an effect the bus holds");
    let sound = Sound::from_samples(&noise(), 2, 48_000).unwrap();
    audio.play(&sound, Play::new().looping(true)).unwrap();
    let ours = mixer.frames(480_000);
    let theirs: Vec<f32> = std::fs::read(theirs)
        .expect("miniaudio's output")
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let (mut signal, mut difference) = (0f64, 0f64);
    for (a, b) in ours.iter().zip(&theirs).skip(4_800 * 2) {
        signal += f64::from(*b).powi(2);
        difference += f64::from(a - b).powi(2);
    }
    let under = 10. * (signal / difference.max(1e-300)).log10();
    println!("  zgx-audio {name}: {under:.1} dB under the signal from miniaudio's");
    assert!(under >= 90., "{name} isn't miniaudio's filter: {under:.1} dB");
}
