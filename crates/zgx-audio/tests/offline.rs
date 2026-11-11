//! The mixer rendered offline and compared with checked-in references
//! (tests/references, 24-bit WAV) to within -90 dB. Every scene is also
//! rendered in blocks of several sizes, which must not change a sample:
//! commands land on their frame, not on a block's edge.
//!
//! `ZGX_AUDIO_BLESS=1 cargo test -p zgx-audio --test offline` writes the
//! references anew; listen to them, and say why they changed.
use std::path::PathBuf;
use zgx_audio::{Audio, Config, Curve, Error, Interpolation, Play, Ramp, Sound, Source, Time, wav};

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
        let (audio, mut mixer) = Audio::offline(config.clone()).unwrap();
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
        std::fs::write(
            &path,
            wav::write(samples, 2, RATE, wav::Encoding::Pcm24).unwrap(),
        )
        .unwrap();
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
fn the_limiter_keeps_low_tones_under_its_ceiling_by_its_gain() {
    // Below about 160 Hz the limiter keeps more gains than above: a tone
    // at 40 Hz and its octaves, far over the ceiling.
    let tones: Vec<Sound> = [40., 60., 80., 120.]
        .map(|hz| sound(&tone(hz, 0.25, RATE), 1, RATE))
        .to_vec();
    let out = render(Config::default(), 12_000, |audio| {
        for tone in &tones {
            audio.play(tone, Play::new().gain(2.)).unwrap();
        }
    });
    assert!(out.iter().all(|s| s.abs() <= 1.), "past the ceiling");
    assert!(out.iter().any(|s| s.abs() > 0.99), "limited below it");
    // Its final clamp keeps any limiter under the ceiling, broken or not:
    // the reference is what shows the gain did the work (the unit tests
    // count what the clamp had to correct).
    reference("limiter_low", &out);
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
    let (audio, mut mixer) = Audio::offline(config).unwrap();
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
    let bytes = wav::write(&[0.5, -0.5, 0.25, -0.25], 2, 22_050, wav::Encoding::Float32).unwrap();
    let audio = Audio::offline(Config::default()).unwrap().0;
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
        commands: 16,
        ..unlimited()
    })
    .unwrap();
    let sent = (0..40)
        .filter(|_| audio.play(&click, Play::new()).is_ok())
        .count();
    // 3 of the 16 hold the default buses.
    assert_eq!(sent, 13);
    mixer.frames(1);
    assert!(audio.play(&click, Play::new()).is_ok());
}

#[test]
fn a_driven_bus_glides_to_each_value_without_a_command() {
    let flat = sound(&[0., 1.], 1, RATE);
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let music = audio.bus("music").unwrap();
    audio
        .play(&flat, Play::new().bus(&music).loop_frames(1..2))
        .unwrap();
    let drive = music.drive();
    mixer.frames(100);
    drive.set(0.25);
    let out = left(&mixer.frames(1000));
    // 10 ms: 480 frames from 1 to 0.25, in straight steps.
    assert_eq!(out[0], 1.);
    assert!((out[240] - 0.625).abs() < 1e-3, "{}", out[240]);
    assert_eq!(out[480], 0.25);
    assert_eq!(out[999], 0.25);
    // Unchanged, nothing moves; a command takes over until it's set again.
    music.set_gain(1., Ramp::NONE).unwrap();
    assert_eq!(left(&mixer.frames(10))[9], 1.);
    drive.set(0.5);
    assert_eq!(left(&mixer.frames(600))[599], 0.5);
}

#[test]
fn with_interface_sounds_off_the_ui_bus_is_silent_and_the_rest_play_on() {
    let flat = sound(&[0., 0.5], 1, RATE);
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let (ui, music) = (audio.bus("ui").unwrap(), audio.bus("music").unwrap());
    audio
        .play(&flat, Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    audio
        .play(&flat, Play::new().bus(&music).loop_frames(1..2).gain(0.5))
        .unwrap();
    mixer.frames(100);
    audio.set_ui_sounds(false).unwrap();
    assert!(!audio.ui_sounds_enabled());
    let out = left(&mixer.frames(1000));
    // After the 5 ms gate, only music's 0.25.
    assert_eq!(out[999], 0.25);
    // The app's own mute and the system's don't undo each other.
    ui.mute(true).unwrap();
    audio.set_ui_sounds(true).unwrap();
    assert_eq!(left(&mixer.frames(1000))[999], 0.25);
    ui.mute(false).unwrap();
    assert_eq!(left(&mixer.frames(1000))[999], 0.75);
}

#[test]
fn ended_voices_are_reported_once() {
    let click = blip();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let a = audio.play(&click, Play::new()).unwrap();
    let b = audio.play(&click, Play::new().at(Time(1000))).unwrap();
    mixer.frames(500);
    assert_eq!(audio.take_ended(), [a.id()]);
    mixer.frames(1000);
    assert_eq!(audio.take_ended(), [b.id()]);
    assert!(audio.take_ended().is_empty());
}

#[test]
fn a_bus_from_another_engine_is_an_error() {
    let click = blip();
    let (a, _) = Audio::offline(unlimited()).unwrap();
    let (b, mut mixer) = Audio::offline(unlimited()).unwrap();
    // A's buses past any B has: as a parent, it was read out of bounds.
    let mut deep = a.master();
    for i in 0..20 {
        deep = a.add_bus(&format!("a{i}"), &deep).unwrap();
    }
    assert!(matches!(b.add_bus("x", &deep), Err(Error::Foreign)));
    // Indices B has too: made B's buses a cycle, and a solo hung the mixer.
    let (a4, a5) = (
        a.add_bus("four", &a.master()).unwrap(),
        a.add_bus("five", &a.master()).unwrap(),
    );
    let b4 = b.add_bus("four", &b.master()).unwrap();
    assert!(matches!(b.add_bus("five", &a5), Err(Error::Foreign)));
    let _ = a4;
    assert!(matches!(
        b.play(&click, Play::new().bus(&deep)),
        Err(Error::Foreign)
    ));
    // B's own still work.
    b.play(&click, Play::new().bus(&b4)).unwrap();
    b4.solo(true).unwrap();
    assert_eq!(left(&mixer.frames(8))[1], 1.);
}

#[test]
fn nothing_but_finite_samples_leave_the_mixer_whatever_comes_in() {
    let wild = [
        0.,
        f32::NAN,
        f32::INFINITY,
        -f32::INFINITY,
        1e30,
        -1e30,
        0.5,
    ];
    let wild = sound(&wild, 1, RATE);
    let tone = sound(&tone(440., 0.1, RATE), 1, RATE);
    for limiter in [None, Some(1.)] {
        let (audio, mut mixer) = Audio::offline(Config {
            limiter,
            ..Config::default()
        })
        .unwrap();
        // A tree as deep as there can be, every bus as loud as it goes.
        let mut deep = audio.master();
        for i in 0..28 {
            deep = audio.add_bus(&format!("deep {i}"), &deep).unwrap();
            deep.set_gain(f32::INFINITY, Ramp::NONE).unwrap();
        }
        for bad in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30] {
            audio
                .play(
                    &wild,
                    Play::new().bus(&deep).gain(bad).pan(bad).looping(true),
                )
                .unwrap();
            let voice = audio
                .play(&tone, Play::new().gain(bad).pitch(bad).pan(bad))
                .unwrap();
            voice.set_gain(bad, Ramp::ms(1.)).unwrap();
            voice.set_pitch(bad, Ramp::ms(1.)).unwrap();
            voice.set_pan(bad, Ramp::ms(1.)).unwrap();
        }
        audio.master().set_gain(f32::NAN, Ramp::ms(2.)).unwrap();
        audio.master().set_gain(1., Ramp::ms(2.)).unwrap();
        let out = mixer.frames(48_000);
        let bad = out.iter().position(|s| !s.is_finite());
        assert_eq!(
            bad, None,
            "limiter {limiter:?}: not finite at sample {bad:?}"
        );
        // A pitch that wasn't a number plays at 1, and its tone ends: left
        // are the four loops and the tone at -inf, kept to 1/64 (6.4 s).
        assert_eq!(audio.stats().voices, 5);
    }
}

#[test]
fn a_sound_of_no_frames_plays_and_ends_at_once() {
    let empty = sound(&[], 2, RATE);
    assert_eq!(empty.frames(), 0);
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let voices = [
        audio.play(&empty, Play::new()).unwrap(),
        audio.play(&empty, Play::new().looping(true)).unwrap(),
        audio
            .play(&empty, Play::new().loop_frames(0..10).pitch(3.))
            .unwrap(),
    ];
    assert!(mixer.frames(64).iter().all(|&s| s == 0.));
    audio.update();
    assert!(voices.iter().all(|v| !v.playing()));
}

#[test]
fn voices_end_and_free_their_slots_while_the_app_makes_no_call() {
    // 400 blips scheduled up front, then not a call on the engine while
    // they play: every one is heard on its frame, none left waiting.
    let click = blip();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    for i in 0..400u64 {
        audio
            .play(&click, Play::new().at(Time(100 + i * 50)))
            .unwrap();
    }
    let out = left(&mixer.frames(100 + 400 * 50 + 100));
    let missing: Vec<u64> = (0..400u64)
        .filter(|i| out[(100 + i * 50 + 1) as usize] != 1.)
        .collect();
    assert_eq!(missing, Vec::<u64>::new(), "blips never heard");
    assert_eq!(audio.stats().voices, 0);
}

#[test]
fn a_stop_for_a_play_still_waiting_for_a_slot_waits_with_it() {
    // One voice: each play of higher priority steals the one before,
    // which fades out in a slot of its own; 17 at once leave the 18th
    // with none until a fade ends. Its stop, sent with it, must still
    // stop it.
    let flat = sound(&[0., 0.5], 1, RATE);
    let (audio, mut mixer) = Audio::offline(Config {
        voices: 1,
        ..unlimited()
    })
    .unwrap();
    let mut last = None;
    for priority in 0..18 {
        last = Some(
            audio
                .play(&flat, Play::new().loop_frames(1..2).priority(priority))
                .unwrap(),
        );
    }
    let last = last.unwrap();
    last.stop().unwrap();
    mixer.frames(4800);
    assert!(
        audio.stats().waited > 0,
        "nothing waited: {:?}",
        audio.stats()
    );
    assert_eq!(audio.stats().voices, 0, "{:?}", audio.stats());
    audio.update();
    assert!(!last.playing());
}

#[test]
fn commands_far_in_the_future_never_hold_back_one_due_now() {
    let flat = sound(&[0., 0.5], 1, RATE);
    let click = blip();
    let (audio, mut mixer) = Audio::offline(Config {
        commands: 64,
        ..unlimited()
    })
    .unwrap();
    let voice = audio.play(&flat, Play::new().loop_frames(1..2)).unwrap();
    mixer.frames(64);
    // Far more than there's room for, an hour away: past some point
    // they're refused, but none blocks what's due now.
    let hour = Time(RATE as u64 * 3600);
    let mut refused = 0;
    for round in 0..4 {
        for _ in 0..40 {
            if audio.play(&click, Play::new().at(hour)).is_err() {
                refused += 1;
            }
        }
        mixer.frames(64 + round);
    }
    assert!(refused > 0, "all taken: no room was ever kept");
    voice.stop().unwrap();
    let out = left(&mixer.frames(1000));
    assert_eq!(out[999], 0., "the stop never came");
    audio.update();
    assert!(!voice.playing());
}

#[test]
fn a_play_four_billion_frames_away_waits_without_hanging_the_mixer() {
    let click = blip();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    for far in [1u64 << 32, (1 << 32) + 1, u64::MAX] {
        audio.play(&click, Play::new().at(Time(far))).unwrap();
    }
    let out = mixer.frames(4096);
    assert!(out.iter().all(|&s| s == 0.));
    assert_eq!(audio.stats().frames, 4096);
}

#[test]
fn a_sound_at_a_rate_the_mixer_can_t_play_is_an_error() {
    for rate in [0, 999, 768_001, u32::MAX] {
        assert!(
            Sound::from_samples(&[0., 0.5], 1, rate).is_err(),
            "{rate} Hz"
        );
    }
    for rate in [1_000, 768_000] {
        assert!(
            Sound::from_samples(&[0., 0.5], 1, rate).is_ok(),
            "{rate} Hz"
        );
    }
}

#[test]
fn a_bus_driven_to_infinity_or_nothing_sends_finite_sound() {
    let flat = sound(&[0., 0.5], 1, RATE);
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let music = audio.bus("music").unwrap();
    audio
        .play(&flat, Play::new().bus(&music).loop_frames(1..2))
        .unwrap();
    // The bus is made with the first block: a drive is followed after.
    mixer.frames(64);
    for bad in [f32::INFINITY, f32::NAN, -f32::INFINITY, 1e30] {
        music.drive().set(bad);
        let out = mixer.frames(1000);
        assert!(out.iter().all(|s| s.is_finite()), "driven to {bad}");
    }
}

#[test]
fn ended_voices_no_one_asks_about_are_not_kept_forever() {
    let click = blip();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let mut played = 0;
    while played < 100_000 {
        for _ in 0..500 {
            if audio.play(&click, Play::new()).is_ok() {
                played += 1;
            }
        }
        mixer.frames(64);
    }
    audio.update();
    let kept = audio.take_ended().len();
    assert!(kept <= 65_536, "{kept} ids kept for no one");
    assert!(kept > 0);
}

#[test]
fn a_time_past_the_clock_s_end_is_its_end() {
    let (audio, _mixer) = Audio::offline(unlimited()).unwrap();
    let never = audio.now() + 1 + audio.seconds(f64::INFINITY);
    assert_eq!(never, Time(u64::MAX));
    assert_eq!(Time(u64::MAX - 1) + 5, Time(u64::MAX));
    // Something due then never sounds, and nothing breaks.
    let tone = sound(&tone(440., 0.1, RATE), 1, RATE);
    audio.play(&tone, Play::new().at(never)).unwrap();
}

#[test]
fn a_voice_s_own_low_pass_is_the_low_pass_effect_s_held_and_moving() {
    use zgx_audio::effects::LowPass;
    // Noise, mono and stereo, through the voice's own low-pass and,
    // apart, through the effect on a bus of its own: the same, still and
    // while the cutoff sweeps from 2 kHz to 300 Hz from frame 1000. (No
    // fade in: the voice's filter comes before its gain, the effect after.)
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let noise: Vec<f32> = (0..9_600)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    for channels in [1, 2] {
        let noise = sound(&noise[..noise.len() / channels * channels], channels, RATE);
        let sweep = Ramp::ms(50.);
        let own = render(unlimited(), 4_000, |audio| {
            let voice = audio
                .play(&noise, Play::new().cutoff(2_000.).fade_in(0.))
                .unwrap();
            voice.at(Time(1_000)).set_cutoff(300., sweep).unwrap();
        });
        let effect = render(unlimited(), 4_000, |audio| {
            let bus = audio.add_bus("filtered", &audio.master()).unwrap();
            let lowpass = bus.add_effect(LowPass::new(2_000.)).unwrap();
            audio
                .play(&noise, Play::new().bus(&bus).fade_in(0.))
                .unwrap();
            lowpass
                .at(Time(1_000))
                .set(LowPass::CUTOFF, 300., sweep)
                .unwrap();
        });
        let worst = own
            .iter()
            .zip(&effect)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1e-6, "{channels} channels: {worst:e} apart");
        // And it does filter: the highs are gone.
        let plain = render(unlimited(), 4_000, |audio| {
            audio.play(&noise, Play::new()).unwrap();
        });
        assert!(own.iter().zip(&plain).any(|(a, b)| (a - b).abs() > 0.05));
    }
}

#[test]
fn a_voice_s_low_pass_left_open_is_no_filter() {
    let tone = sound(&tone(9_000., 0.1, RATE), 1, RATE);
    let plain = render(unlimited(), 4_000, |audio| {
        audio.play(&tone, Play::new()).unwrap();
    });
    let open = render(unlimited(), 4_000, |audio| {
        // Past the top is the top; closed and opened again, it ends open.
        let voice = audio.play(&tone, Play::new().cutoff(30_000.)).unwrap();
        voice
            .at(Time(3_000))
            .set_cutoff(f32::NAN, Ramp::NONE)
            .unwrap();
    });
    assert_eq!(plain, open);
}

#[test]
fn a_voice_s_low_pass_closes_from_open_along_its_ramp() {
    use zgx_audio::effects::LowPass;
    // Open, then down to 300 Hz over 50 ms from frame 1000: once the
    // effect's history and the voice's have died away, the same.
    let tone = sound(&tone(3_000., 0.2, RATE), 1, RATE);
    let sweep = Ramp::ms(50.);
    let own = left(&render(unlimited(), 6_000, |audio| {
        let voice = audio.play(&tone, Play::new().fade_in(0.)).unwrap();
        voice.at(Time(1_000)).set_cutoff(300., sweep).unwrap();
    }));
    let effect = left(&render(unlimited(), 6_000, |audio| {
        let bus = audio.add_bus("filtered", &audio.master()).unwrap();
        let lowpass = bus.add_effect(LowPass::new(20_000.)).unwrap();
        audio
            .play(&tone, Play::new().bus(&bus).fade_in(0.))
            .unwrap();
        lowpass
            .at(Time(1_000))
            .set(LowPass::CUTOFF, 300., sweep)
            .unwrap();
    }));
    // Down, the 3 kHz tone is 20 dB and more under what it was open.
    let level = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        level(&own[4_000..4_400]) < 0.1 * level(&own[400..800]),
        "it never closed"
    );
    let worst = own[5_000..]
        .iter()
        .zip(&effect[5_000..])
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-5, "{worst:e} apart");
}

#[test]
fn a_voice_sends_to_other_buses_at_levels_of_its_own() {
    // A constant 0.25 on master, sent at 0.5 to a bus whose gain is 2: the
    // master hears 0.25 and 0.25; the send ramped to 0 over 100 frames from
    // frame 1,000, it's gone, and a third send takes its place.
    let flat = sound(&[0., 0.25], 1, RATE);
    let out = left(&render(unlimited(), 3_000, |audio| {
        let aux = audio.add_bus("aux", &audio.master()).unwrap();
        let other = audio.add_bus("other", &audio.master()).unwrap();
        let third = audio.add_bus("third", &audio.master()).unwrap();
        aux.set_gain(2., Ramp::NONE).unwrap();
        let voice = audio
            .play(&flat, Play::new().loop_frames(1..2).send(&aux, 0.5))
            .unwrap();
        voice
            .at(Time(500))
            .set_send(&other, 1., Ramp::NONE)
            .unwrap();
        // Two already: ignored.
        voice
            .at(Time(600))
            .set_send(&third, 1., Ramp::NONE)
            .unwrap();
        voice
            .at(Time(1_000))
            .set_send(&aux, 0., Ramp::seconds(100. / f64::from(RATE)))
            .unwrap();
        // Now there's room.
        voice
            .at(Time(2_000))
            .set_send(&third, 0.5, Ramp::NONE)
            .unwrap();
    }));
    assert_eq!(out[400], 0.5, "own and sent");
    assert_eq!(out[700], 0.75, "and another, not a third");
    assert_eq!(
        out[1_050],
        0.25 + 0.25 + 0.25 * 0.5 * 2. * 0.5,
        "halfway down"
    );
    assert_eq!(out[1_500], 0.5, "the first gone");
    assert_eq!(out[2_500], 0.625, "a third in its place");
}

#[test]
fn a_play_s_sends_keep_to_two_as_a_voice_s_do() {
    // Sends to two buses, then a third (ignored, as `set_send` ignores
    // it), then the first again at another level (which sets its level).
    let flat = sound(&[0., 0.25], 1, RATE);
    let out = left(&render(unlimited(), 200, |audio| {
        let buses: Vec<_> = ["one", "two", "three"]
            .iter()
            .map(|name| audio.add_bus(name, &audio.master()).unwrap())
            .collect();
        let play = Play::new()
            .loop_frames(1..2)
            .send(&buses[0], 0.5)
            .send(&buses[1], 1.)
            .send(&buses[2], 2.)
            .send(&buses[0], 0.25);
        audio.play(&flat, play).unwrap();
    }));
    // Its own 0.25, the first's 0.0625 and the second's 0.25.
    assert_eq!(out[100], 0.25 + 0.25 * 0.25 + 0.25);
}

#[test]
fn a_send_to_another_engine_s_bus_is_refused() {
    let (audio, _mixer) = Audio::offline(unlimited()).unwrap();
    let (other, _other_mixer) = Audio::offline(unlimited()).unwrap();
    let flat = sound(&[0., 0.25], 1, RATE);
    let theirs = other.add_bus("theirs", &other.master()).unwrap();
    assert_eq!(
        audio
            .play(&flat, Play::new().send(&theirs, 1.))
            .unwrap_err(),
        Error::Foreign
    );
    let voice = audio.play(&flat, Play::new()).unwrap();
    assert_eq!(
        voice.set_send(&theirs, 1., Ramp::NONE).unwrap_err(),
        Error::Foreign
    );
}

#[test]
fn a_send_for_a_play_still_waiting_for_a_slot_waits_with_it() {
    // One voice: 17 plays at once leave the 18th waiting for a slot (as
    // a stop's test does). Its send, set as it's sent, must reach it.
    let flat = sound(&[0., 0.25], 1, RATE);
    let (audio, mut mixer) = Audio::offline(Config {
        voices: 1,
        ..unlimited()
    })
    .unwrap();
    let aux = audio.add_bus("aux", &audio.master()).unwrap();
    let mut last = None;
    for priority in 0..18 {
        last = Some(
            audio
                .play(&flat, Play::new().loop_frames(1..2).priority(priority))
                .unwrap(),
        );
    }
    last.unwrap().set_send(&aux, 1., Ramp::NONE).unwrap();
    let out = left(&mixer.frames(4_800));
    assert!(audio.stats().waited > 0);
    assert_eq!(out[4_799], 0.5, "it plays, and sends");
}

#[test]
fn a_sound_started_partway_plays_from_that_frame() {
    // A ramp, each frame its own value. Started 1,000 frames in, it plays
    // what a start at 0 plays 1,000 frames on, after the millisecond's
    // fade a start away from zero gets.
    let ramp: Vec<f32> = (0..4_000).map(|i| i as f32 / 8_000.).collect();
    let ramp = sound(&ramp, 1, RATE);
    let fade = 2 * RATE as usize / 1000;
    let whole = render(unlimited(), 4_000, |audio| {
        audio.play(&ramp, Play::new()).unwrap();
    });
    let partway = render(unlimited(), 3_000, |audio| {
        audio.play(&ramp, Play::new().offset(1_000)).unwrap();
    });
    assert_eq!(partway[0], 0., "it fades in");
    assert!(partway[2] > 0.);
    assert_eq!(&partway[fade..], &whole[2_000 + fade..]);
    // Looping, 10,000 frames in is 2,000 round.
    let round = render(unlimited(), 2_000, |audio| {
        audio
            .play(&ramp, Play::new().looping(true).offset(10_000))
            .unwrap();
    });
    assert_eq!(round[0], 0., "it fades in from where it starts round");
    assert_eq!(&round[fade..], &whole[4_000 + fade..]);
    // A one-shot started past its end ends at once.
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let voice = audio.play(&ramp, Play::new().offset(4_000)).unwrap();
    assert!(mixer.frames(512).iter().all(|&s| s == 0.));
    audio.update();
    assert!(!voice.playing());
}

#[test]
fn a_metered_bus_says_its_level_after_its_gain() {
    // 0.25 on both sides into music at gain 0.5: 0.125 out. Two seconds
    // in, its RMS is that (its 300 ms average long settled) and its peak
    // too; asked again with nothing played since, the peak is 0. An
    // unmetered bus says nothing. Master's, after its limiter, is under
    // its ceiling with a sound twice full scale.
    let (audio, mut mixer) = Audio::offline(Config::default()).unwrap();
    let music = audio.bus("music").unwrap();
    let ui = audio.bus("ui").unwrap();
    music.set_gain(0.5, Ramp::NONE).unwrap();
    music.set_metered(true).unwrap();
    audio.master().set_metered(true).unwrap();
    let flat = sound(&[0., 0.25], 1, RATE);
    audio
        .play(&flat, Play::new().bus(&music).loop_frames(1..2))
        .unwrap();
    let loud = sound(&[0., 2.], 1, RATE);
    audio
        .play(&loud, Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    mixer.frames(2 * RATE as usize);
    let level = music.level();
    assert!((level.rms - 0.125).abs() < 1e-4, "{level:?}");
    assert_eq!(level.peak, 0.125);
    assert_eq!(music.level().peak, 0., "read: none since");
    assert_eq!(ui.level(), zgx_audio::Level::default(), "not metered");
    let master = audio.master().level();
    assert!(master.peak > 0.5 && master.peak <= 1., "{master:?}");
    // Stopped measuring: nothing.
    music.set_metered(false).unwrap();
    mixer.frames(512);
    assert_eq!(music.level(), zgx_audio::Level::default());
}
