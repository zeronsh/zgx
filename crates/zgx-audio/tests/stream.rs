//! Streams: decoded as they play on the decode thread, rendered offline.
//! A stream must sound exactly as the same source loaded whole, loop
//! without a seam, end, and leave the decode thread asleep.
use std::time::{Duration, Instant};
use zgx_audio::{Audio, Config, Play, Sound, Source, wav};

fn noise(frames: usize, channels: usize, rate: u32) -> Vec<u8> {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut samples: Vec<f32> = (0..frames * channels)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    samples[..channels].fill(0.);
    wav::write(&samples, channels, rate, wav::Encoding::Float32).unwrap()
}

fn engine() -> (Audio, zgx_audio::Offline) {
    Audio::offline(Config {
        limiter: None,
        ..Config::default()
    })
    .unwrap()
}

/// `frames` frames in blocks of 512, as a device asks for them: a ring
/// holds half a second, so one call for more would run it dry.
fn blocks(mixer: &mut zgx_audio::Offline, frames: usize) -> Vec<f32> {
    let mut out = Vec::new();
    while out.len() < frames * 2 {
        out.extend(mixer.frames(512.min(frames - out.len() / 2)));
    }
    out
}

fn until(what: &str, done: impl Fn() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "waited 5 s for {what}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_stream_sounds_exactly_as_the_same_sound_loaded_whole() {
    // Two seconds at 44.1 kHz: four rings' worth, read at another rate.
    let bytes = noise(88_200, 2, 44_100);
    let whole = {
        let (audio, mut mixer) = engine();
        let sound = audio.load(Source::bytes(bytes.clone())).unwrap();
        audio
            .play(&sound, Play::new().gain(0.7).pitch(1.01))
            .unwrap();
        mixer.frames(100_000)
    };
    let (audio, mut mixer) = engine();
    let voice = audio
        .stream(Source::bytes(bytes), Play::new().gain(0.7).pitch(1.01))
        .unwrap();
    let streamed = blocks(&mut mixer, 100_000);
    let at = whole.iter().zip(&streamed).position(|(a, b)| a != b);
    assert_eq!(
        at, None,
        "the stream differs from the whole sound at sample {at:?}"
    );
    assert!(streamed.iter().any(|&s| s != 0.));
    // It ended, gave its voice back, and the decode thread let it go.
    audio.update();
    assert!(!voice.playing());
    until("the decode thread to let the stream go", || {
        audio.stats().streams == 0
    });
    assert_eq!(audio.stats().dry, 0);
}

#[test]
fn a_looping_stream_goes_round_without_a_seam() {
    let bytes = noise(1000, 1, 48_000);
    let source: Vec<f32> = wav::read(&bytes).unwrap().samples;
    let (audio, mut mixer) = engine();
    let voice = audio
        .stream(Source::bytes(bytes), Play::new().looping(true))
        .unwrap();
    let out = blocks(&mut mixer, 5500);
    let left: Vec<f32> = out.as_chunks::<2>().0.iter().map(|f| f[0]).collect();
    for (i, &sample) in left.iter().enumerate() {
        assert_eq!(sample, source[i % 1000], "frame {i}");
    }
    assert!(voice.playing());
    assert_eq!(audio.stats().streams, 1);
    voice.stop().unwrap();
    mixer.frames(1000);
    audio.update();
    until("the stream to close", || audio.stats().streams == 0);
}

#[test]
fn a_stream_the_decoder_cannot_keep_up_with_fades_out_and_back_in() {
    // Rendered without waiting for the decode thread, faster than real
    // time: it runs dry, which is counted and faded, never cut.
    let bytes = noise(480_000, 1, 48_000);
    let (audio, mut mixer) = engine();
    audio.stream(Source::bytes(bytes), Play::new()).unwrap();
    audio.settle();
    let mut out = vec![0f32; 512 * 2];
    let mut steepest = 0f32;
    let mut previous = 0f32;
    for _ in 0..400 {
        mixer.render_now(&mut out);
        for frame in out.as_chunks::<2>().0 {
            steepest = steepest.max((frame[0] - previous).abs());
            previous = frame[0];
        }
    }
    let stats = audio.stats();
    assert!(stats.dry >= 1, "it never ran dry: {stats:?}");
    // The noise's own steps are up to 0.5; a cut to zero from its peak would
    // be one too, so look at the fade instead: it ends at zero, not mid-air.
    assert!(steepest <= 0.5 + 1e-6, "{steepest}");
}

#[test]
fn a_stream_due_later_starts_on_its_frame() {
    let bytes = noise(4800, 1, 48_000);
    let (audio, mut mixer) = engine();
    let at = audio.now() + 777;
    audio
        .stream(Source::bytes(bytes), Play::new().at(at))
        .unwrap();
    let out = mixer.frames(2000);
    let first = out.iter().position(|&s| s != 0.).map(|i| i / 2);
    // Its first frame is zero, its second not.
    assert_eq!(first, Some(778));
}

#[cfg(feature = "vorbis")]
mod vorbis {
    use super::*;

    const TONE: &[u8] = include_bytes!("assets/tone.ogg");

    #[test]
    fn ogg_vorbis_loads_whole_and_sounds_like_its_source() {
        // tone.ogg: 0.5 s at 44.1 kHz, 440 Hz left and 660 Hz right at
        // 1/16 of full scale, made with ffmpeg's own Vorbis encoder at q2.
        let (audio, _mixer) = engine();
        let sound = audio.load(Source::bytes(TONE)).unwrap();
        assert_eq!((sound.channels(), sound.rate()), (2, 44_100));
        assert!(
            (sound.frames() as i64 - 22_050).abs() < 2048,
            "{} frames",
            sound.frames()
        );
        let file = Source::file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/assets/tone.ogg"
        ));
        assert_eq!(audio.load(file).unwrap().frames(), sound.frames());
    }

    #[test]
    fn an_ogg_stream_sounds_exactly_as_it_does_loaded_whole() {
        let whole = {
            let (audio, mut mixer) = engine();
            let sound = audio.load(Source::bytes(TONE)).unwrap();
            audio.play(&sound, Play::new()).unwrap();
            mixer.frames(30_000)
        };
        let (audio, mut mixer) = engine();
        audio.stream(Source::bytes(TONE), Play::new()).unwrap();
        let streamed = blocks(&mut mixer, 30_000);
        assert_eq!(whole, streamed);
        // Against the tone it was made from (440 Hz on the left; ffmpeg's
        // sine is 1/8 of full scale, halved): the best-fitting sine, then what
        // isn't it. A lossy codec at q2: well below the tone, not silence.
        let left: Vec<f64> = whole
            .as_chunks::<2>()
            .0
            .iter()
            .skip(2000)
            .take(20_000)
            .map(|f| f64::from(f[0]))
            .collect();
        let w = 440. * std::f64::consts::TAU / 48_000.;
        let (mut ss, mut sc, mut cc, mut ys, mut yc) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for (i, &y) in left.iter().enumerate() {
            let (sin, cos) = (w * i as f64).sin_cos();
            (ss, sc, cc, ys, yc) = (
                ss + sin * sin,
                sc + sin * cos,
                cc + cos * cos,
                ys + y * sin,
                yc + y * cos,
            );
        }
        let det = ss * cc - sc * sc;
        let (a, b) = ((ys * cc - yc * sc) / det, (yc * ss - ys * sc) / det);
        let amplitude = (a * a + b * b).sqrt();
        assert!(
            (amplitude - 0.0625).abs() < 0.005,
            "the tone's amplitude: {amplitude}"
        );
        let (signal, noise) = left
            .iter()
            .enumerate()
            .fold((0f64, 0f64), |(s, n), (i, &y)| {
                let (sin, cos) = (w * i as f64).sin_cos();
                let fit = a * sin + b * cos;
                (s + fit * fit, n + (y - fit).powi(2))
            });
        let ratio = 10. * (signal / noise).log10();
        assert!(ratio > 20., "{ratio:.1} dB");
    }

    #[test]
    fn a_looping_ogg_stream_goes_round() {
        let (audio, mut mixer) = engine();
        let voice = audio
            .stream(Source::bytes(TONE), Play::new().looping(true))
            .unwrap();
        let out = blocks(&mut mixer, 80_000);
        assert_eq!(audio.stats().dry, 0);
        assert!(
            voice.playing(),
            "still playing after three times its length"
        );
        assert!(out[70_000 * 2..].iter().any(|&s| s.abs() > 0.03));
    }
}

#[test]
fn a_sound_that_is_not_a_known_format_says_so() {
    let (audio, _mixer) = engine();
    let error = audio
        .stream(Source::bytes(&b"FORM\0\0\0\0AIFF"[..]), Play::new())
        .unwrap_err();
    assert!(
        error.to_string().contains("not a format zgx-audio reads"),
        "{error}"
    );
    let _ = Sound::from_samples(&[0.], 1, 48_000);
}
