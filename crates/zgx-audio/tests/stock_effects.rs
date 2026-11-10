//! The stock effects through the whole engine: each one's response to an
//! impulse and to a sweep, rendered offline in blocks of several sizes
//! (which must give the same samples) and compared with a checked-in
//! reference (tests/references, 24-bit WAV) to within -90 dB; then what
//! each must do, measured.
//!
//! `ZGX_AUDIO_BLESS=1 cargo test -p zgx-audio --test stock_effects` writes
//! the references anew; say why they changed.
use std::f64::consts::PI;
use std::path::PathBuf;
use zgx_audio::effects::{
    Compressor, Delay, Duck, FLAT, HighPass, LowPass, Peaking, Reverb, Slope,
};
use zgx_audio::{Audio, Bus, Config, Effect, Play, Ramp, Sound, Time, wav};

const RATE: u32 = 48_000;
/// -90 dB, as a difference of samples.
const TOLERANCE: f32 = 3.162e-5;
/// Frames of impulse response, then of sweep.
const IMPULSE: usize = 2048;
const SWEEP: usize = 4096;

fn unlimited() -> Config {
    Config {
        limiter: None,
        ..Config::default()
    }
}

/// An impulse of half scale (room for a peak's gain under the references'
/// full scale) on the left at frame 1 and on the right at frame 101 (a
/// sound's first frame is zero, so it gets no fade in).
fn impulse() -> Sound {
    let mut samples = vec![0f32; 512 * 2];
    samples[2] = 0.5;
    samples[101 * 2 + 1] = 0.5;
    Sound::from_samples(&samples, 2, RATE).unwrap()
}

/// A logarithmic sweep from 20 Hz to 20 kHz, a fifth of full scale (room
/// for a resonance or a peak's gain under the references' full scale), on
/// the left; the same, inverted, on the right.
fn sweep() -> Sound {
    let (low, high) = (20f64, 20_000f64);
    let n = SWEEP as f64;
    let k = (high / low).ln();
    let samples: Vec<f32> = (0..SWEEP)
        .flat_map(|i| {
            let t = i as f64 / f64::from(RATE);
            let duration = n / f64::from(RATE);
            let phase = 2. * PI * low * duration / k * ((t / duration * k).exp() - 1.);
            let s = (phase.sin() * 0.2) as f32;
            [s, -s]
        })
        .collect();
    Sound::from_samples(&samples, 2, RATE).unwrap()
}

/// The impulse, then the sweep, through what `add` puts on master, in
/// blocks of several sizes, which must agree.
fn respond(add: impl Fn(&Audio, &Bus)) -> Vec<f32> {
    let (impulse, sweep) = (impulse(), sweep());
    let mut first: Option<Vec<f32>> = None;
    for block in [512, 1, 37, 128, 300] {
        let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
        let master = audio.master();
        add(&audio, &master);
        audio.play(&impulse, Play::new()).unwrap();
        audio
            .play(&sweep, Play::new().at(Time(IMPULSE as u64)))
            .unwrap();
        let mut out = vec![0.; (IMPULSE + SWEEP) * 2];
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
    first.unwrap()
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
            |(at, worst), (i, d)| {
                if d > worst { (i, d) } else { (at, worst) }
            },
        );
    assert!(
        worst <= TOLERANCE,
        "{name}: frame {} differs by {:.1} dB from its reference",
        at / 2,
        20. * worst.log10()
    );
}

fn channel(samples: &[f32], c: usize) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[c]).collect()
}

/// The gain of `effect` at `hz`, in decibels: a long sine through it on
/// master, measured once it has settled.
fn gain_at(effect: impl Effect + Clone, hz: f64) -> f64 {
    let frames = 48_000;
    let tone: Vec<f32> = (0..frames)
        .map(|i| (2. * PI * hz * i as f64 / f64::from(RATE)).sin() as f32 * 0.25)
        .collect();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    audio.master().add_effect(effect).unwrap();
    let tone = Sound::from_samples(&tone, 1, RATE).unwrap();
    audio.play(&tone, Play::new()).unwrap();
    let left = channel(&mixer.frames(frames), 0);
    // A whole number of cycles over the last half second.
    let (mut re, mut im) = (0f64, 0f64);
    for (i, &s) in left.iter().enumerate().skip(frames / 2) {
        let phase = 2. * PI * hz * i as f64 / f64::from(RATE);
        re += f64::from(s) * phase.sin();
        im += f64::from(s) * phase.cos();
    }
    let amplitude = 2. * (re * re + im * im).sqrt() / (frames / 2) as f64;
    20. * (amplitude / 0.25).log10()
}

#[test]
fn low_pass_filters_respond_as_references_and_fall_12_or_24_db_an_octave() {
    let out = respond(|_, master| {
        master.add_effect(LowPass::new(1000.)).unwrap();
    });
    reference("lowpass_12", &out);
    // The impulse response sums to the gain at 0 Hz (1) times the
    // impulse's (0.5).
    let sum: f32 = channel(&out, 0)[..IMPULSE].iter().sum();
    assert!((sum - 0.5).abs() < 1e-3, "{sum}");
    let out = respond(|_, master| {
        master
            .add_effect(LowPass::new(1000.).slope(Slope::Db24))
            .unwrap();
    });
    reference("lowpass_24", &out);
    // -3 dB at the cutoff for both (Butterworth); then 12 or 24 dB an
    // octave, two octaves on.
    for (slope, per_octave) in [(Slope::Db12, 12.), (Slope::Db24, 24.)] {
        let at = gain_at(LowPass::new(1000.).slope(slope), 1000.);
        assert!(
            (at + 3.01).abs() < 0.05,
            "{slope:?} at its cutoff: {at:.2} dB"
        );
        // Two to four times the cutoff: past the knee, short of where the
        // bilinear transform's warping near half the rate steepens it.
        let fall = gain_at(LowPass::new(1000.).slope(slope), 2000.)
            - gain_at(LowPass::new(1000.).slope(slope), 4000.);
        assert!(
            (fall - per_octave).abs() < 1.,
            "{slope:?}: {fall:.1} dB from 2 to 4 kHz"
        );
    }
}

#[test]
fn a_high_pass_responds_as_its_reference_and_passes_nothing_steady() {
    let out = respond(|_, master| {
        master
            .add_effect(HighPass::new(200.).resonance(2.))
            .unwrap();
    });
    reference("highpass_12", &out);
    let sum: f32 = channel(&out, 0)[..IMPULSE].iter().sum();
    assert!(sum.abs() < 1e-3, "{sum}");
    // Resonance 2: +6 dB at its cutoff (20 log Q).
    let at = gain_at(HighPass::new(200.).resonance(2.), 200.);
    assert!((at - 6.02).abs() < 0.05, "{at:.2} dB");
    let below = gain_at(HighPass::new(200.), 25.);
    assert!(below < -35., "{below:.1} dB three octaves down");
}

#[test]
fn a_peaking_band_lifts_its_centre_by_its_gain_and_leaves_the_rest() {
    let out = respond(|_, master| {
        master.add_effect(Peaking::new(1000., 9., 1.4)).unwrap();
    });
    reference("peaking", &out);
    let centre = gain_at(Peaking::new(1000., 9., 1.4), 1000.);
    assert!((centre - 9.).abs() < 0.02, "{centre:.3} dB");
    let cut = gain_at(Peaking::new(1000., -12., 0.7), 1000.);
    assert!((cut + 12.).abs() < 0.02, "{cut:.3} dB");
    for hz in [30., 15_000.] {
        let away = gain_at(Peaking::new(1000., 9., 1.4), hz);
        assert!(away.abs() < 0.2, "{away:.2} dB at {hz} Hz");
    }
}

#[test]
fn a_cutoff_moving_a_frame_at_a_time_lands_on_its_frames_at_any_block_size() {
    let out = respond(|_, master| {
        let filter = master.add_effect(LowPass::new(200.).resonance(4.)).unwrap();
        filter
            .at(Time(IMPULSE as u64))
            .set(
                LowPass::CUTOFF,
                8000.,
                Ramp::seconds(SWEEP as f64 / f64::from(RATE)),
            )
            .unwrap();
        filter
            .at(Time((IMPULSE + SWEEP / 2) as u64))
            .set(LowPass::RESONANCE, FLAT, Ramp::ms(20.))
            .unwrap();
    });
    reference("lowpass_moving", &out);
    assert!(out.iter().all(|s| s.is_finite() && s.abs() < 4.));
}

#[test]
fn a_ringing_filter_runs_on_after_its_sound_until_its_tail_is_quiet() {
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let world = audio.bus("world").unwrap();
    // Q 10 at 100 Hz rings for 10.36 * 10 / (pi * 100) = 0.33 s to -90 dB.
    world.add_effect(LowPass::new(100.).resonance(10.)).unwrap();
    audio.play(&impulse(), Play::new().bus(&world)).unwrap();
    let left = channel(&mixer.frames(48_000), 0);
    // The impulse's sound is 512 frames; the filter rings on, till its
    // tail has passed (15 840 frames after the sound), and no further.
    let late = left[15_000..15_100]
        .iter()
        .fold(0f32, |m, s| m.max(s.abs()));
    assert!(late > 0., "silent before its tail was over");
    let peak = left.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(late < peak * 3.2e-4, "{late} still loud, against {peak}");
    assert!(left[17_000..].iter().all(|&s| s == 0.), "run past its tail");
}

#[test]
fn a_delay_echoes_on_its_frames_each_echo_its_feedback_as_loud() {
    // 10 ms (480 frames), half as loud each time, the echoes at half.
    let out = respond(|_, master| {
        master
            .add_effect(Delay::new(0.01, 0.05).feedback(0.5).mix(0.5, 1.))
            .unwrap();
    });
    reference("delay", &out);
    let left = channel(&out, 0);
    // The impulse (0.5) on frame 1, then 0.25, 0.125, 0.0625 on its frames.
    assert_eq!(left[1], 0.5);
    for (k, want) in [(1, 0.25), (2, 0.125), (3, 0.0625)] {
        assert_eq!(left[1 + 480 * k], want, "echo {k}");
        assert_eq!(left[480 * k], 0., "before echo {k}");
    }
    // Shorter than a block: the runs split at the delay, and the echoes
    // still land on their frames.
    let short = respond(|_, master| {
        master
            .add_effect(Delay::new(0.0005, 0.01).feedback(0.5).mix(1., 0.))
            .unwrap();
    });
    let left = channel(&short, 0);
    assert_eq!(
        (left[1], left[25], left[49], left[73]),
        (0., 0.5, 0.25, 0.125)
    );
}

#[test]
fn a_delay_s_time_glides_without_a_click_at_any_block_size() {
    let out = respond(|_, master| {
        let echo = master
            .add_effect(Delay::new(0.01, 0.05).feedback(0.3).mix(0.7, 1.))
            .unwrap();
        echo.at(Time(IMPULSE as u64))
            .set(
                Delay::TIME,
                0.03,
                Ramp::seconds(SWEEP as f64 / f64::from(RATE)),
            )
            .unwrap();
    });
    reference("delay_moving", &out);
    assert!(out.iter().all(|s| s.is_finite() && s.abs() < 1.));
}

#[test]
fn a_delay_rings_on_until_its_echoes_are_quiet() {
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let world = audio.bus("world").unwrap();
    // 0.1 s apart, feedback 0.5: 4.5 / log10(2) = 15 echoes to -90 dB.
    world
        .add_effect(Delay::new(0.1, 0.2).feedback(0.5).mix(1., 1.))
        .unwrap();
    audio.play(&impulse(), Play::new().bus(&world)).unwrap();
    let left = channel(&mixer.frames(96_000), 0);
    assert!(left[1 + 4800 * 12].abs() > 0., "the 12th echo");
    assert!(left[78_000..].iter().all(|&s| s == 0.), "run past its tail");
}

/// The reverb's response to the impulse (on the left only), wet alone.
fn reverb_impulse(reverb: Reverb, frames: usize) -> Vec<f32> {
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    audio.master().add_effect(reverb.mix(1., 0.)).unwrap();
    let mut left = vec![0f32; 512 * 2];
    left[2] = 0.5;
    let click = Sound::from_samples(&left, 2, RATE).unwrap();
    audio.play(&click, Play::new()).unwrap();
    mixer.frames(frames)
}

/// Decibels of `samples`' energy over a window from `at`.
fn level_db(samples: &[f32], at: usize, window: usize) -> f64 {
    let e: f64 = samples[at..at + window]
        .iter()
        .map(|&s| f64::from(s).powi(2))
        .sum();
    10. * (e / window as f64).log10()
}

#[test]
fn a_reverb_falls_60_db_in_its_decay_after_its_pre_delay() {
    let out = reverb_impulse(Reverb::new(1., 1.).damping(0.).pre_delay(0.02), 96_000);
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    // Nothing before the pre-delay (960 frames) and the shortest line
    // (1427 frames) have passed, from the impulse on frame 1.
    let first = left.iter().position(|&s| s != 0.).unwrap();
    assert_eq!(first, 1 + 960 + 1427, "its first sound");
    // The slope of its energy: from 0.2 s to 0.7 s it falls 30 dB at a
    // decay of 1 s (60 dB a second), within 15%.
    let both: Vec<f32> = left.iter().zip(&right).map(|(l, r)| l + r).collect();
    let fall = level_db(&both, 9_600, 4_800) - level_db(&both, 33_600, 4_800);
    assert!((fall - 30.).abs() < 4.5, "{fall:.1} dB over half a second");
    // Left and right are two different echoes of one room.
    let (mut lr, mut ll, mut rr) = (0f64, 0f64, 0f64);
    for (l, r) in left.iter().zip(&right).skip(4_800).take(24_000) {
        lr += f64::from(*l) * f64::from(*r);
        ll += f64::from(*l).powi(2);
        rr += f64::from(*r).powi(2);
    }
    let correlation = lr / (ll * rr).sqrt();
    assert!(
        correlation.abs() < 0.3,
        "left and right correlate {correlation:.2}"
    );
}

#[test]
fn a_damped_reverb_loses_its_highs_before_its_lows() {
    // The share of its energy above about 6 kHz (sample to sample
    // differences against the samples), early and late.
    let share = |samples: &[f32], at: usize| {
        let window = &samples[at..at + 4_800];
        let diff: f64 = window
            .windows(2)
            .map(|w| f64::from(w[1] - w[0]).powi(2))
            .sum();
        let all: f64 = window.iter().map(|&s| f64::from(s).powi(2)).sum();
        diff / all
    };
    let damped = channel(&reverb_impulse(Reverb::new(1., 2.).damping(0.8), 48_000), 0);
    let bright = channel(&reverb_impulse(Reverb::new(1., 2.).damping(0.), 48_000), 0);
    let (early, late) = (share(&damped, 4_800), share(&damped, 33_600));
    assert!(
        late < early * 0.5,
        "damped: highs {early:.3} early, {late:.3} late"
    );
    let (early, late) = (share(&bright, 4_800), share(&bright, 33_600));
    assert!(
        late > early * 0.7,
        "undamped: highs {early:.3} early, {late:.3} late"
    );
}

#[test]
fn a_reverb_responds_as_its_references_its_size_and_decay_moving_on_their_frames() {
    let out = respond(|_, master| {
        master
            .add_effect(Reverb::new(0.5, 0.3).pre_delay(0.005).mix(0.5, 1.))
            .unwrap();
    });
    reference("reverb", &out);
    let out = respond(|_, master| {
        let room = master
            .add_effect(Reverb::new(0.5, 0.3).pre_delay(0.005).mix(0.5, 1.))
            .unwrap();
        let sweep = Ramp::seconds(SWEEP as f64 / f64::from(RATE));
        room.at(Time(IMPULSE as u64))
            .set(Reverb::SIZE, 1.5, sweep)
            .unwrap();
        room.at(Time(IMPULSE as u64))
            .set(Reverb::DECAY, 2., sweep)
            .unwrap();
        room.at(Time((IMPULSE + 100) as u64))
            .set(Reverb::DAMPING, 0.9, Ramp::ms(30.))
            .unwrap();
    });
    reference("reverb_moving", &out);
    assert!(out.iter().all(|s| s.is_finite() && s.abs() < 1.));
}

#[test]
fn a_reverb_rings_on_after_its_sound_and_rests_after_its_tail() {
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let world = audio.bus("world").unwrap();
    // 0.4 s to fall 60 dB: 0.6 s to 90 dB, after 10 ms and its longest line.
    world
        .add_effect(Reverb::new(1., 0.4).pre_delay(0.01).mix(1., 0.))
        .unwrap();
    audio.play(&impulse(), Play::new().bus(&world)).unwrap();
    let left = channel(&mixer.frames(96_000), 0);
    assert!(
        left[20_000..20_100].iter().any(|&s| s != 0.),
        "silent too soon"
    );
    assert!(left[40_000..].iter().all(|&s| s == 0.), "run past its tail");
}

/// A steady sine of `db` decibels (peak) through `effect` on master, and
/// its level (peak, dB) once settled.
fn level_through(effect: impl Effect + Clone, db: f64) -> f64 {
    let amplitude = 10f64.powf(db / 20.);
    let tone: Vec<f32> = (0..48_000)
        .map(|i| ((2. * PI * 1000. * i as f64 / f64::from(RATE)).sin() * amplitude) as f32)
        .collect();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    audio.master().add_effect(effect).unwrap();
    let tone = Sound::from_samples(&tone, 1, RATE).unwrap();
    audio.play(&tone, Play::new().fade_in(0.)).unwrap();
    let left = channel(&mixer.frames(48_000), 0);
    let peak = left[36_000..].iter().fold(0f32, |m, s| m.max(s.abs()));
    20. * f64::from(peak).log10()
}

#[test]
fn a_compressor_s_curve_is_its_ratio_above_its_threshold_through_its_knee() {
    // -30 dB, 4 to 1, a 6 dB knee; an attack quick enough to reach each
    // peak's reduction and a release slow enough to hold it through the
    // cycle (a slower attack settles short of the curve on a sine, as a
    // detector smoothed in decibels does).
    let make = || Compressor::new(-30., 4.).times(0.0001, 1.);
    for (input, want) in [
        (-40., -40.),           // below the knee: untouched
        (-24., -30. + 6. / 4.), // above it: a quarter of what's over
        (-6., -30. + 24. / 4.),
        (-30., -30. - 3. * 0.75 * 3. / 6. / 2.), // the knee's middle
    ] {
        let got = level_through(make(), input);
        assert!(
            (got - want).abs() < 0.1,
            "{input} dB in: {got:.2} out, not {want:.2}"
        );
    }
    // Makeup lifts it all.
    let got = level_through(make().knee(6., 6.), -6.);
    assert!((got - (-24. + 6.)).abs() < 0.1, "{got:.2}");
}

#[test]
fn a_compressor_takes_its_attack_to_come_down_and_its_release_to_come_back() {
    // A burst 20 dB over the threshold, then quiet again.
    let (attack, release) = (0.01, 0.1);
    let samples: Vec<f32> = (0..48_000)
        .map(|i| {
            let loud = if (4_800..14_400).contains(&i) {
                1.
            } else {
                0.01
            };
            ((2. * PI * 1000. * i as f64 / f64::from(RATE)).sin() * loud) as f32
        })
        .collect();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    audio
        .master()
        .add_effect(
            Compressor::new(-20., 20.)
                .times(attack, release)
                .knee(0., 0.),
        )
        .unwrap();
    let sound = Sound::from_samples(&samples, 1, RATE).unwrap();
    audio.play(&sound, Play::new().fade_in(0.)).unwrap();
    let left = channel(&mixer.frames(48_000), 0);
    let peak = |from: usize| {
        20. * f64::from(
            left[from..from + 48]
                .iter()
                .fold(0f32, |m, s| m.max(s.abs())),
        )
        .log10()
    };
    // An attack's time in, two thirds of the way down (1 - 1/e).
    let (start, settled) = (peak(4_800), peak(13_000));
    let at = peak(4_800 + 480);
    let fraction = (start - at) / (start - settled);
    assert!(
        (0.5..0.8).contains(&fraction),
        "{fraction:.2} of the way down after its attack"
    );
    // After the burst the quiet tone comes back over the release: still
    // held down a release in, and whole after five.
    let quiet = 20. * 0.01f64.log10();
    assert!(peak(14_400 + 480) < quiet - 3., "back too soon");
    assert!((peak(14_400 + 5 * 4_800) - quiet).abs() < 0.5, "not back");
}

#[test]
fn a_compressor_responds_as_its_reference_its_threshold_moving_on_its_frames() {
    let out = respond(|_, master| {
        let squeeze = master
            .add_effect(Compressor::new(-24., 6.).times(0.001, 0.05))
            .unwrap();
        squeeze
            .at(Time(IMPULSE as u64))
            .set(
                Compressor::THRESHOLD,
                -40.,
                Ramp::seconds(SWEEP as f64 / f64::from(RATE)),
            )
            .unwrap();
    });
    reference("compressor", &out);
}

#[test]
fn a_bus_ducks_under_another_while_it_sounds_and_comes_back_after() {
    let flat = Sound::from_samples(&[0., 0.5], 1, RATE).unwrap();
    let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
    let (music, dialogue) = (
        audio.bus("music").unwrap(),
        audio.add_bus("dialogue", &audio.master()).unwrap(),
    );
    audio
        .play(&flat, Play::new().bus(&music).loop_frames(1..2))
        .unwrap();
    // No side chain, nothing: added keyed on nothing.
    music.add_effect(Duck::new(-12., 0.01, 0.1)).unwrap();
    music.duck(&dialogue, Duck::new(-12., 0.01, 0.1)).unwrap();
    let before = channel(&mixer.frames(4_800), 0);
    assert_eq!(before[4_799], 0.5, "ducked with no dialogue");
    let line = audio
        .play(
            &flat,
            Play::new().bus(&dialogue).loop_frames(1..2).gain(0.1),
        )
        .unwrap();
    let during = channel(&mixer.frames(9_600), 0);
    // Music down 12 dB (a quarter), with the dialogue's 0.05 on top.
    let want = 0.5 * 10f32.powf(-12. / 20.) + 0.05;
    assert!(
        (during[9_599] - want).abs() < 1e-3,
        "{} not {want}",
        during[9_599]
    );
    line.stop().unwrap();
    let after = channel(&mixer.frames(48_000), 0);
    assert!(
        (after[47_999] - 0.5).abs() < 1e-3,
        "{} not back",
        after[47_999]
    );
}

#[cfg(feature = "hrtf")]
mod binaural {
    use super::*;
    use std::sync::Arc;
    use zgx_audio::effects::{BLOCK, Binaural, HrtfSet};

    const TAPS: usize = 300;

    /// Four directions (ahead, right, behind, left), each ear a response of
    /// its own: decaying noise, so any mix-up shows.
    fn set() -> (Arc<HrtfSet>, Vec<[Vec<f32>; 2]>) {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut noise = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        let pairs: Vec<[Vec<f32>; 2]> = (0..4)
            .map(|_| {
                [0, 1].map(|_| {
                    (0..TAPS)
                        .map(|k| noise() * (-(k as f32) / 80.).exp() * 0.3)
                        .collect()
                })
            })
            .collect();
        let directions = vec![[0., 0., -1.], [1., 0., 0.], [0., 0., 1.], [-1., 0., 0.]];
        let set = HrtfSet::new(
            RATE,
            directions,
            pairs.iter().map(|p| p[0].clone()).collect(),
            pairs.iter().map(|p| p[1].clone()).collect(),
        )
        .unwrap();
        (Arc::new(set), pairs)
    }

    fn noise_sound(frames: usize) -> (Sound, Vec<f32>) {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let samples: Vec<f32> = (0..frames)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
            })
            .collect();
        let mut played = samples.clone();
        played[0] = 0.;
        let mut with_zero = samples;
        with_zero[0] = 0.;
        (Sound::from_samples(&with_zero, 1, RATE).unwrap(), played)
    }

    /// `input` through `response`, a block late, at frame `t`.
    fn direct(input: &[f32], response: &[f32], t: usize) -> f32 {
        let mut sum = 0f64;
        for (k, &h) in response.iter().enumerate() {
            if let Some(i) = t.checked_sub(BLOCK + k) {
                sum += f64::from(h) * f64::from(input.get(i).copied().unwrap_or(0.));
            }
        }
        sum as f32
    }

    /// The noise heard through `build`'s binaural effect on master, in
    /// blocks of several sizes, which must agree.
    fn render(frames: usize, build: impl Fn(&Audio, Arc<HrtfSet>)) -> Vec<f32> {
        let (set, _) = set();
        let (sound, _) = noise_sound(frames);
        let mut first: Option<Vec<f32>> = None;
        for block in [512, 1, 37, 128, 300] {
            let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
            build(&audio, set.clone());
            audio.play(&sound, Play::new()).unwrap();
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
        first.unwrap()
    }

    #[test]
    fn each_ear_hears_the_direction_s_response_a_block_late() {
        let frames = 4_096;
        let (_, pairs) = set();
        let (_, input) = noise_sound(frames);
        for (azimuth, pair) in [(0., 0), (85., 1), (-170., 2), (-95., 3)] {
            let out = render(frames, |audio, set| {
                audio
                    .master()
                    .add_effect(Binaural::new(set).facing(azimuth, 0.))
                    .unwrap();
            });
            for (side, response) in pairs[pair].iter().enumerate() {
                let heard = channel(&out, side);
                let worst = (0..frames)
                    .map(|t| (heard[t] - direct(&input, response, t)).abs())
                    .fold(0f32, f32::max);
                assert!(
                    worst < 1e-5,
                    "{azimuth} degrees, ear {side}: off by {worst:e}"
                );
            }
        }
    }

    #[test]
    fn a_new_direction_crossfades_in_over_a_block() {
        let frames = 4_096;
        let (_, pairs) = set();
        let (_, input) = noise_sound(frames);
        // Ahead, then right from frame 1000: the block that ends after it
        // fades from one to the other, and the next is the new one's.
        let out = render(frames, |audio, set| {
            let ears = audio.master().add_effect(Binaural::new(set)).unwrap();
            ears.at(Time(1_000))
                .set(Binaural::AZIMUTH, 90., Ramp::NONE)
                .unwrap();
        });
        let left = channel(&out, 0);
        // The effect starts on the first sound, frame 1 (the noise's first
        // sample is zero), so its blocks end at 1 + 128k. The block over
        // 897..=1024 decides at its end, where the azimuth is 90: its
        // output (1025..1153) fades from the old to the new.
        let fading = (1_000 - 1) / BLOCK * BLOCK + 1 + BLOCK;
        for t in [1_100, 1_150] {
            let (old, new) = (
                direct(&input, &pairs[0][0], t),
                direct(&input, &pairs[1][0], t),
            );
            let fade = (t - fading + 1) as f32 / BLOCK as f32;
            let want = old + (new - old) * fade;
            assert!((left[t] - want).abs() < 1e-5, "{t}: {} not {want}", left[t]);
        }
        for t in [1_200, 2_000, 4_000] {
            let want = direct(&input, &pairs[1][0], t);
            assert!((left[t] - want).abs() < 1e-5, "{t}: {} not {want}", left[t]);
        }
    }

    #[test]
    fn a_set_at_another_rate_keeps_its_loudness_and_its_length() {
        // A millisecond's response at 96 and at 24 kHz, an average after a
        // silent onset (as a head's is, before the sound reaches the ear):
        // a level of 0.5 must come out 0.5 and 0.25 at 48, and the tail
        // must be the millisecond and two blocks at the output's rate.
        for rate in [96_000u32, 24_000] {
            let taps = rate as usize / 1_000;
            let average = |gain: f32| -> Vec<f32> {
                (0..taps)
                    .map(|k| if k < 4 { 0. } else { gain / (taps - 4) as f32 })
                    .collect()
            };
            let (left, right) = (average(1.), average(0.5));
            let set =
                Arc::new(HrtfSet::new(rate, vec![[0., 0., -1.]], vec![left], vec![right]).unwrap());
            let level = Sound::from_samples(&[0.5; 64], 1, RATE).unwrap();
            let (audio, mut mixer) = Audio::offline(unlimited()).unwrap();
            audio
                .master()
                .add_effect(Binaural::new(set.clone()))
                .unwrap();
            audio
                .play(&level, Play::new().looping(true).fade_in(0.))
                .unwrap();
            let out = mixer.frames(2_048);
            let [l, r] = [out[4_000], out[4_001]];
            assert!(
                (l - 0.5).abs() < 0.005 && (r - 0.25).abs() < 0.0025,
                "{rate}: {l}, {r}"
            );
            let mut ears = Binaural::new(set);
            ears.prepare(
                zgx_audio::Format {
                    rate: RATE,
                    channels: 2,
                },
                512,
            );
            let want = 0.001 + 2. * BLOCK as f64 / f64::from(RATE);
            let tail = ears.tail().as_secs_f64();
            assert!(
                (tail - want).abs() < 1e-6,
                "{rate}: a tail of {tail}, not {want}"
            );
        }
    }
}
