//! Emitters in space, rendered offline: a tone flying past the listener
//! must rise and fall in pitch as Doppler says, cross from left to right,
//! swell and fade as its fall-off says and dull with the air; and a short
//! fly-by, timed on the clock, must come out the same at any block size and
//! as its reference (tests/references, -90 dB).
//!
//! `ZGX_AUDIO_BLESS=1 cargo test -p zgx-audio --test spatial` writes the
//! reference anew.
use std::path::PathBuf;
use zgx_audio::effects::LowPass;
use zgx_audio::spatial::{Emitter, Falloff, Listener, Vec3, air_cutoff, doppler};
use zgx_audio::{Audio, Config, PanLaw, Play, Sound, Time, wav};

const RATE: u32 = 48_000;
/// -90 dB, as a difference of samples.
const TOLERANCE: f32 = 3.162e-5;
/// 400 Hz: 120 frames a cycle, so a second of it loops without a seam.
const HZ: f64 = 400.;

fn unlimited() -> Config {
    Config {
        limiter: None,
        ..Config::default()
    }
}

fn tone() -> Sound {
    let samples: Vec<f32> = (0..RATE as usize)
        .map(|i| ((2. * std::f64::consts::PI * HZ * i as f64 / f64::from(RATE)).sin() * 0.5) as f32)
        .collect();
    Sound::from_samples(&samples, 1, RATE).unwrap()
}

/// An emitter of the tone on a bus of its own with a low-pass for the air,
/// played equal-power, falling off inversely from 2 m to 100 m.
fn emitter(audio: &Audio, tone: &Sound) -> Emitter {
    let bus = audio.add_bus("emitter", &audio.master()).unwrap();
    let air = bus.add_effect(LowPass::new(20_000.)).unwrap();
    let voice = audio
        .play(
            tone,
            Play::new()
                .bus(&bus)
                .looping(true)
                .law(PanLaw::EqualPower)
                .fade_in(0.),
        )
        .unwrap();
    let mut emitter = Emitter::new(voice, Falloff::inverse(2., 100.));
    emitter.air = Some(air);
    emitter
}

fn channel(samples: &[f32], c: usize) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[c]).collect()
}

/// Its frequency over `window` frames from `at`: rising zero crossings
/// over the time between the first and last.
fn frequency(samples: &[f32], at: usize, window: usize) -> f64 {
    let rising: Vec<f64> = (at..at + window)
        .filter(|&i| samples[i - 1] < 0. && samples[i] >= 0.)
        .map(|i| {
            // Where between the frames it crossed.
            let (a, b) = (f64::from(samples[i - 1]), f64::from(samples[i]));
            (i - 1) as f64 + a / (a - b)
        })
        .collect();
    let (first, last) = (rising[0], rising[rising.len() - 1]);
    (rising.len() - 1) as f64 * f64::from(RATE) / (last - first)
}

fn power_db(samples: &[f32], at: usize, window: usize) -> f64 {
    let e: f64 = samples[at * 2..(at + window) * 2]
        .iter()
        .map(|&s| f64::from(s).powi(2))
        .sum();
    10. * (e / window as f64).log10()
}

#[test]
fn a_tone_flying_past_rises_and_falls_crosses_over_and_swells_with_its_distance() {
    let tone = tone();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let emitter = emitter(&audio, &tone);
    let listener = Listener::default();
    // 4 m ahead, from 40 m left to 40 m right at 40 m/s: two seconds.
    let (speed, update) = (40f32, 480usize);
    let at = |frame: usize| -> Vec3 { [-40. + speed * frame as f32 / RATE as f32, 0., -4.] };
    let velocity = [speed, 0., 0.];
    let mut out = Vec::new();
    for k in 0..200 {
        let frame = k * update;
        emitter
            .update(
                at(frame + update),
                velocity,
                &listener,
                update as f64 / f64::from(RATE),
            )
            .unwrap();
        out.extend(mixer.frames(update));
    }
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    // Pitch: before the pass, coming (higher); after, going (lower); each
    // as doppler() says where it was, within 0.3%.
    let still = [0.; 3];
    for (from, window) in [(9_600, 2_400), (84_000, 2_400)] {
        let middle = from + window / 2;
        let want = HZ * f64::from(doppler(at(middle), velocity, still, still, 1.));
        let got = frequency(&left, from, window);
        assert!(
            ((got - want) / want).abs() < 0.003,
            "{from}: {got:.2} Hz, not {want:.2}"
        );
    }
    assert!(frequency(&left, 9_600, 2_400) > HZ && frequency(&left, 84_000, 2_400) < HZ);
    // Across: left first, right after.
    let side = |c: &[f32], from: usize| -> f64 {
        c[from..from + 2_400]
            .iter()
            .map(|&s| f64::from(s).powi(2))
            .sum()
    };
    assert!(side(&left, 9_600) > 10. * side(&right, 9_600), "left first");
    assert!(
        side(&right, 84_000) > 10. * side(&left, 84_000),
        "right after"
    );
    // Distance: at its closest (4 m, gain 0.5) and 30 m off (gain 2/30.3),
    // the power equal-power keeps whatever the pan, the difference is the
    // fall-off's.
    let near = power_db(&out, 47_000, 2_000);
    let far = power_db(&out, 9_600, 2_000);
    let want = 20. * (0.5f64 / (2. / f64::from(30.2f32.hypot(4.)))).log10();
    assert!(
        ((near - far) - want).abs() < 0.5,
        "{:.2} dB, not {want:.2}",
        near - far
    );
}

#[test]
fn a_fly_by_timed_on_the_clock_is_the_same_at_any_block_size_and_its_reference() {
    let tone = tone();
    let frames = 8_192;
    let mut first: Option<Vec<f32>> = None;
    for block in [512, 1, 37, 128, 300] {
        let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
        let emitter = emitter(&audio, &tone);
        let listener = Listener::default();
        // 1 m ahead, from 8 m left to 8 m right at 94 m/s, an update every
        // 256 frames, all sent before it plays.
        for k in 0..32u64 {
            let t = (k * 256) as f32 / RATE as f32;
            let position = [-8. + 94. * t, 0., -1.];
            emitter
                .update_at(
                    Time(k * 256),
                    position,
                    [94., 0., 0.],
                    &listener,
                    256. / f64::from(RATE),
                )
                .unwrap();
        }
        let mut out = vec![0.; frames * 2];
        for chunk in out.chunks_mut(block * 2) {
            mixer.render(chunk);
        }
        match &first {
            None => first = Some(out),
            Some(first) => {
                let at = first.iter().zip(&out).position(|(a, b)| a != b);
                assert_eq!(at, None, "blocks of {block} differ from 512's at {at:?}");
            }
        }
    }
    let out = first.unwrap();
    assert!(out.iter().all(|s| s.is_finite() && s.abs() < 1.));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/references/flyby.wav");
    if std::env::var_os("ZGX_AUDIO_BLESS").is_some() {
        std::fs::write(
            &path,
            wav::write(&out, 2, RATE, wav::Encoding::Pcm24).unwrap(),
        )
        .unwrap();
        return;
    }
    let expected = wav::read(&std::fs::read(&path).expect("ZGX_AUDIO_BLESS=1 writes it")).unwrap();
    assert_eq!(expected.samples.len(), out.len());
    let worst = expected
        .samples
        .iter()
        .zip(&out)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(
        worst <= TOLERANCE,
        "{:.1} dB from its reference",
        20. * worst.log10()
    );
}

#[test]
fn the_air_takes_the_highs_of_a_far_emitter_as_air_cutoff_says() {
    // 8 kHz (6 frames a cycle: a second loops cleanly), held still at
    // 10 m and then at 80 m: beyond the fall-off, the far one loses what a
    // 12 dB low-pass at each one's air cutoff takes at 8 kHz.
    let high: Vec<f32> = (0..RATE as usize)
        .map(|i| {
            ((2. * std::f64::consts::PI * 8_000. * i as f64 / f64::from(RATE)).sin() * 0.5) as f32
        })
        .collect();
    let high = Sound::from_samples(&high, 1, RATE).unwrap();
    let level = |distance: f32| {
        let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
        let emitter = emitter(&audio, &high);
        emitter
            .update([0., 0., -distance], [0.; 3], &Listener::default(), 0.)
            .unwrap();
        let out = mixer.frames(24_000);
        power_db(&out, 12_000, 6_000)
    };
    let falloff = Falloff::inverse(2., 100.);
    let spread = 20. * f64::from(falloff.gain(10.) / falloff.gain(80.)).log10();
    let air = |d: f32| 10. * (1. + (8_000. / f64::from(air_cutoff(d))).powi(4)).log10();
    let want = spread + air(80.) - air(10.);
    let got = level(10.) - level(80.);
    // The filter's bilinear transform warps near 8 kHz: within 1 dB.
    assert!((got - want).abs() < 1., "{got:.2} dB, not {want:.2}");
    assert!(got > spread + 3., "the air took {:.2} dB", got - spread);
}

#[test]
fn a_voice_s_pan_law_shares_it_by_balance_or_by_equal_power() {
    let flat = Sound::from_samples(&[0., 0.5], 1, RATE).unwrap();
    let at = |law: PanLaw, pan: f32| {
        let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
        audio
            .play(&flat, Play::new().loop_frames(1..2).law(law).pan(pan))
            .unwrap();
        let out = mixer.frames(64);
        [out[126], out[127]]
    };
    assert_eq!(
        at(PanLaw::Balance, 0.),
        [0.5, 0.5],
        "a balance: whole in the middle"
    );
    let [l, r] = at(PanLaw::EqualPower, 0.);
    assert!((l - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && l == r);
    let [l, r] = at(PanLaw::EqualPower, 0.5);
    assert!((l * l + r * r - 0.25).abs() < 1e-6, "the power kept");
    assert!(at(PanLaw::EqualPower, 1.)[0].abs() < 1e-7);
}
