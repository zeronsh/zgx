//! The mixer rendered offline and compared with checked-in references
//! (tests/references, 24-bit WAV) to within -90 dB. Every scene is also
//! rendered in blocks of several sizes, which must not change a sample:
//! commands land on their frame, not on a block's edge.
//!
//! `ZGX_AUDIO_BLESS=1 cargo test -p zgx-audio --test offline` writes the
//! references anew; listen to them, and say why they changed.
use std::path::PathBuf;
use zgx_audio::{Audio, Config, Curve, Interpolation, Play, Ramp, Sound, Source, Time, wav};

const RATE: u32 = 48_000;

/// -90 dB, as a difference of samples.
const TOLERANCE: f32 = 3.162e-5;

fn tone(frequency: f32, seconds: f32, rate: u32) -> Vec<f32> {
    let frames = (seconds * rate as f32) as usize;
    (0..frames)
        .map(|i| (i as f32 * frequency * std::f32::consts::TAU / rate as f32).sin() * 0.5)
        .collect()
}

fn sound(samples: &[f32], channels: usize, rate: u32) -> Sound {
    Sound::from_samples(samples, channels, rate).unwrap()
}

/// A click that starts at zero: no fade in is put on it.
fn blip() -> Sound {
    sound(&[0., 1., -0.5, 0.25, -0.125], 1, RATE)
}

/// Renders `frames` frames of the scene `build` sets up (all its commands
/// timed), in blocks of several sizes, checks they agree, and returns them.
fn render(config: Config, frames: usize, build: impl Fn(&Audio)) -> Vec<f32> {
    let mut first: Option<Vec<f32>> = None;
    for block in [512, 1, 37, 128, 300] {
        let (audio, mut mixer) = Audio::offline(config.clone());
        build(&audio);
        let mut out = vec![0.; frames * mixer.channels()];
        for chunk in out.chunks_mut(block * mixer.channels()) {
            mixer.render(chunk);
        }
        match &first {
            None => first = Some(out),
            Some(first) => {
                let at = first.iter().zip(&out).position(|(a, b)| a != b);
                assert_eq!(
                    at, None,
                    "blocks of {block} frames differ from 512's at sample {at:?}"
                );
            }
        }
    }
    first.unwrap()
}

fn unlimited() -> Config {
    Config {
        limiter: None,
        ..Config::default()
    }
}

/// Compares stereo `samples` with the reference `name`.
fn reference(name: &str, samples: &[f32]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/references")
        .join(format!("{name}.wav"));
    if std::env::var_os("ZGX_AUDIO_BLESS").is_some() {
        std::fs::write(&path, wav::write(samples, 2, RATE, wav::Encoding::Pcm24)).unwrap();
        return;
    }
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("{} is missing: ZGX_AUDIO_BLESS=1 writes it", path.display()));
    let expected = wav::read(&bytes).unwrap();
    assert_eq!((expected.channels, expected.rate), (2, RATE));
    assert_eq!(
        expected.samples.len(),
        samples.len(),
        "{name}: a different length"
    );
    let (at, worst) = expected
        .samples
        .iter()
        .zip(samples)
        .map(|(a, b)| (a - b).abs())
        .enumerate()
        .fold(
            (0, 0f32),
            |(at, worst), (i, d)| if d > worst { (i, d) } else { (at, worst) },
        );
    assert!(
        worst <= TOLERANCE,
        "{name}: frame {} differs by {:.1} dB from its reference",
        at / 2,
        20. * worst.log10()
    );
}

fn left(samples: &[f32]) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[0]).collect()
}

fn first_sound(samples: &[f32]) -> Option<usize> {
    samples.iter().position(|&s| s != 0.).map(|i| i / 2)
}

#[test]
fn sounds_start_on_their_frame() {
    let click = blip();
    let out = render(unlimited(), 1024, |audio| {
        for at in [100, 300, 301, 777] {
            audio.play(&click, Play::new().at(Time(at))).unwrap();
        }
    });
    // The blip's first sample is zero, its second whole.
    let left = left(&out);
    assert_eq!(first_sound(&out), Some(101));
    assert_eq!(&left[100..106], &[0., 1., -0.5, 0.25, -0.125, 0.]);
    assert_eq!(&left[300..307], &[0., 1., 0.5, -0.25, 0.125, -0.125, 0.]);
    assert_eq!(left[778], 1.);
    reference("schedule", &out);
}

#[test]
fn a_sound_away_from_zero_fades_in_over_a_millisecond() {
    let flat = sound(&[0.5; 480], 1, RATE);
    let out = left(&render(unlimited(), 600, |audio| {
        audio.play(&flat, Play::new()).unwrap();
    }));
    assert_eq!(out[0], 0.);
    assert_eq!(out[24], 0.25);
    assert_eq!(out[48], 0.5);
    assert_eq!(out[479], 0.5);
    assert_eq!(out[480], 0.);
}

#[test]
fn gain_pan_and_pitch_move_without_steps_and_stops_fade() {
    let tone = sound(&tone(440., 0.2, RATE), 1, RATE);
    let out = render(unlimited(), 4320, |audio| {
        let voice = audio.play(&tone, Play::new().gain(0.8)).unwrap();
        voice.at(Time(480)).set_gain(0.2, Ramp::ms(10.)).unwrap();
        voice.at(Time(1200)).set_pan(-1., Ramp::ms(15.)).unwrap();
        voice.at(Time(2400)).set_pitch(1.5, Ramp::ms(25.)).unwrap();
        voice
            .at(Time(3600))
            .stop_after(Ramp::ms(10.).curve(Curve::EqualPower))
            .unwrap();
    });
    // Panned hard left, the right side is silent.
    let right: Vec<f32> = out.as_chunks::<2>().0.iter().map(|f| f[1]).collect();
    assert!(right[1920..2400].iter().all(|&s| s == 0.));
    // Nothing after the stop's 10 ms.
    assert!(out[(3600 + 480) * 2..].iter().all(|&s| s == 0.));
    // No step anywhere bigger than the tone's own (0.5 * 2π * 440 * 1.5 / 48000).
    let left = left(&out);
    let steepest = left
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0f32, f32::max);
    assert!(steepest < 0.044, "{steepest}");
    reference("voice_ramps", &out);
}

#[test]
fn buses_mute_solo_and_fade_along_their_curves() {
    let low = sound(&tone(220., 0.5, RATE), 1, RATE);
    let high = sound(&tone(660., 0.5, RATE), 1, RATE);
    let out = render(unlimited(), 5600, |audio| {
        let (ui, music) = (audio.bus("ui").unwrap(), audio.bus("music").unwrap());
        let duck = audio.add_bus("duck", &music).unwrap();
        audio.play(&low, Play::new().bus(&ui).gain(0.5)).unwrap();
        audio.play(&high, Play::new().bus(&duck).gain(0.5)).unwrap();
        // Soloing the child: its parent stays audible, ui goes quiet.
        duck.at(Time(1000)).solo(true).unwrap();
        duck.at(Time(2000)).solo(false).unwrap();
        music.at(Time(3000)).mute(true).unwrap();
        ui.at(Time(4000))
            .fade_to(0., 0.02, Curve::Exponential)
            .unwrap();
    });
    // After the solo's 5 ms (240 frames) only the soloed tone is left.
    let solo = &out[(1000 + 240) * 2..2000 * 2];
    let only_high = left(&render(unlimited(), 2000, |audio| {
        audio.play(&high, Play::new().gain(0.5)).unwrap();
    }));
    for (i, pair) in solo.as_chunks::<2>().0.iter().enumerate() {
        assert!((pair[0] - only_high[1240 + i]).abs() < 1e-6);
    }
    // The fade to zero ends in silence, and music is muted: nothing at all.
    assert!(out[(4000 + 960) * 2..].iter().all(|&s| s == 0.));
    reference("bus_controls", &out);
}

#[test]
fn every_fade_curve_reaches_its_target_by_its_own_path() {
    let flat = sound(&[0., 0.8], 1, RATE);
    for (curve, halfway) in [
        (Curve::Linear, 0.4),
        (Curve::EqualPower, 0.8 * 0.5f32.sqrt()),
        (Curve::Exponential, 0.8 * 1e-2),
    ] {
        let out = left(&render(unlimited(), 2000, |audio| {
            let world = audio.bus("world").unwrap();
            world.set_gain(1e-4, Ramp::NONE).unwrap();
            audio
                .play(&flat, Play::new().bus(&world).loop_frames(1..2))
                .unwrap();
            world
                .at(Time(100))
                .fade_to(1., 1000. / RATE as f64, curve)
                .unwrap();
        }));
        // Exponential, from -80 dB to 0 dB, is at -40 dB halfway.
        let mid = out[600];
        assert!(
            (mid - halfway).abs() < halfway * 1e-3,
            "{curve:?}: {mid} halfway, not {halfway}"
        );
        assert_eq!(out[1100], 0.8, "{curve:?} lands on its target");
    }
}

#[test]
fn the_limiter_keeps_master_under_its_ceiling() {
    let tones: Vec<Sound> = (0..8)
        .map(|i| sound(&tone(200. + 50. * i as f32, 0.1, RATE), 1, RATE))
        .collect();
    let out = render(Config::default(), 2400, |audio| {
        for tone in &tones {
            audio.play(tone, Play::new()).unwrap();
        }
    });
    assert!(out.iter().all(|s| s.abs() <= 1.), "past the ceiling");
    assert!(
        out.iter().any(|s| s.abs() > 0.99),
        "limited to the ceiling, not below it"
    );
    reference("limiter", &out);
}

#[test]
fn sounds_at_other_rates_play_at_their_own_speed() {
    let slow = sound(&tone(1000., 0.01, 24_000), 1, 24_000);
    let wide = sound(
        &[0., 0., 0.1, -0.1, 0.2, -0.2, 0.3, -0.3, 0., 0.],
        2,
        96_000,
    );
    for (interpolation, name) in [
        (Interpolation::Linear, "resample_linear"),
        (Interpolation::Cubic, "resample_cubic"),
    ] {
        let out = render(
            Config {
                interpolation,
                ..unlimited()
            },
            960,
            |audio| {
                audio.play(&slow, Play::new()).unwrap();
                audio.play(&wide, Play::new().at(Time(600))).unwrap();
            },
        );
        // 240 frames at 24 kHz last 480 at 48 kHz.
        let left = left(&out);
        assert!(left[470] != 0. && left[480..600].iter().all(|&s| s == 0.));
        // 96 kHz, two channels: every other frame, each side its own.
        assert_eq!(&out[600 * 2..603 * 2], &[0., 0., 0.2, -0.2, 0., 0.]);
        reference(name, &out);
    }
}

/// The level of `frequency` in `samples` (one channel at [`RATE`]), as
/// the amplitude of a sine.
fn level(samples: &[f32], frequency: f64) -> f64 {
    let (mut re, mut im) = (0f64, 0f64);
    for (i, &s) in samples.iter().enumerate() {
        let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(RATE);
        re += f64::from(s) * phase.cos();
        im += f64::from(s) * phase.sin();
    }
    2. * (re * re + im * im).sqrt() / samples.len() as f64
}

#[test]
fn resampling_keeps_high_tones_and_adds_little_else() {
    // A 15 kHz tone at 44.1 kHz, played at 48 kHz: what's left of it, and
    // the image the interpolator makes at 44.1 - 15 = 29.1 kHz, folded back
    // to 48 - 29.1 = 18.9 kHz.
    let tone = sound(&tone(15_000., 0.5, 44_100), 1, 44_100);
    let mut found = Vec::new();
    for interpolation in [Interpolation::Linear, Interpolation::Cubic] {
        let out = render(
            Config {
                interpolation,
                ..unlimited()
            },
            19_200,
            |audio| {
                audio.play(&tone, Play::new()).unwrap();
            },
        );
        let left = &left(&out)[4800..19_200];
        let kept = 20. * (level(left, 15_000.) / 0.5).log10();
        let image = 20. * (level(left, 18_900.) / 0.5).log10();
        found.push((interpolation, kept, image));
    }
    let [(_, linear_kept, linear_image), (_, cubic_kept, cubic_image)] = found[..] else {
        unreachable!()
    };
    // Measured: linear keeps -3.4 dB of it, its image -15.0 dB; cubic keeps
    // -1.7 dB, image -15.5 dB. A 4-point cubic flattens the band; it
    // doesn't filter much (a windowed sinc would).
    assert!(
        linear_kept > -3.6 && linear_image < -14.5,
        "linear: {linear_kept:.1} dB, image {linear_image:.1} dB"
    );
    assert!(
        cubic_kept > -1.8 && cubic_image < -15.,
        "cubic: {cubic_kept:.1} dB, image {cubic_image:.1} dB"
    );
}

#[test]
fn loops_go_round_their_points_until_stopped() {
    let ramp: Vec<f32> = (0..100).map(|i| i as f32 / 200.).collect();
    let ramp = sound(&ramp, 1, RATE);
    let out = left(&render(unlimited(), 600, |audio| {
        let voice = audio.play(&ramp, Play::new().loop_frames(40..60)).unwrap();
        voice.at(Time(300)).stop().unwrap();
    }));
    // An intro to frame 60, then 40..60 again and again.
    assert_eq!(out[59], 59. / 200.);
    assert_eq!(out[60], 40. / 200.);
    assert_eq!(out[80], 40. / 200.);
    assert_eq!(out[299], 59. / 200.);
    assert!(out[300 + 240..].iter().all(|&s| s == 0.));
}

#[test]
fn past_the_voice_limit_the_quietest_of_the_lowest_priority_goes() {
    let flat = sound(&[0., 0.1], 1, RATE);
    let config = Config {
        voices: 2,
        ..unlimited()
    };
    let (audio, mut mixer) = Audio::offline(config);
    let quiet = audio
        .play(&flat, Play::new().loop_frames(1..2).gain(0.1))
        .unwrap();
    let loud = audio
        .play(&flat, Play::new().loop_frames(1..2).gain(1.))
        .unwrap();
    mixer.frames(100);
    let important = audio
        .play(&flat, Play::new().loop_frames(1..2).gain(0.05).priority(1))
        .unwrap();
    mixer.frames(500);
    assert_eq!(audio.stats().stolen, 1);
    assert!(!quiet.playing() && loud.playing() && important.playing());
    // Too quiet, and no higher: it never starts.
    let unheard = audio.play(&flat, Play::new().gain(0.01)).unwrap();
    mixer.frames(100);
    assert_eq!((audio.stats().stolen, audio.stats().dropped), (1, 1));
    assert!(!unheard.playing());
}

#[test]
fn a_voice_stopped_before_it_starts_makes_no_sound() {
    let click = blip();
    let out = render(unlimited(), 1000, |audio| {
        let voice = audio.play(&click, Play::new().at(Time(500))).unwrap();
        voice.stop().unwrap();
    });
    assert_eq!(first_sound(&out), None);
}

#[test]
fn wav_bytes_load_at_their_own_rate_and_channels() {
    let bytes = wav::write(&[0.5, -0.5, 0.25, -0.25], 2, 22_050, wav::Encoding::Float32);
    let audio = Audio::offline(Config::default()).0;
    let sound = audio.load(Source::bytes(bytes)).unwrap();
    assert_eq!(
        (sound.frames(), sound.channels(), sound.rate()),
        (2, 2, 22_050)
    );
    let error = audio
        .load(Source::file("/nonexistent/click.wav"))
        .unwrap_err();
    assert!(
        error.to_string().starts_with("/nonexistent/click.wav: "),
        "{error}"
    );
    assert!(audio.load(Source::bytes(&b"OggS"[..])).is_err());
}

#[test]
fn a_full_queue_is_an_error_not_a_wait() {
    let click = blip();
    let (audio, mut mixer) = Audio::offline(Config {
        commands: 8,
        ..unlimited()
    });
    let sent = (0..20)
        .filter(|_| audio.play(&click, Play::new()).is_ok())
        .count();
    // 3 of the 8 hold the default buses.
    assert_eq!(sent, 5);
    mixer.frames(1);
    assert!(audio.play(&click, Play::new()).is_ok());
}
