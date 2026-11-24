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

#[test]
fn a_stream_takes_a_bus_and_values_as_a_sound_does() {
    let (audio, mut mixer) = engine();
    let (other, _) = engine();
    let bytes = noise(4_800, 2, 48_000);
    let foreign = other.bus("music").unwrap();
    assert!(matches!(
        audio.stream(Source::bytes(bytes.clone()), Play::new().bus(&foreign)),
        Err(zgx_audio::Error::Foreign)
    ));
    let wild = Play::new()
        .gain(f32::INFINITY)
        .pan(f32::NAN)
        .pitch(f32::NAN);
    audio.stream(Source::bytes(bytes), wild).unwrap();
    audio.settle();
    let out = blocks(&mut mixer, 4_800);
    assert!(out.iter().all(|s| s.is_finite()));
    assert!(out.iter().any(|&s| s != 0.), "it played");
}

/// The test's Ogg Vorbis with its header saying `rate`, its page's
/// checksum made again (CRC-32, polynomial 0x04c11db7, unreflected).
#[cfg(feature = "vorbis")]
fn ogg_at(rate: u32) -> Vec<u8> {
    let mut bytes = include_bytes!("assets/tone.ogg").to_vec();
    // One segment: the identification packet starts at 28, its rate at 40.
    bytes[40..44].copy_from_slice(&rate.to_le_bytes());
    let page = 27
        + usize::from(bytes[26])
        + bytes[27..27 + usize::from(bytes[26])]
            .iter()
            .map(|&b| usize::from(b))
            .sum::<usize>();
    bytes[22..26].fill(0);
    let mut crc = 0u32;
    for &byte in &bytes[..page] {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    bytes[22..26].copy_from_slice(&crc.to_le_bytes());
    bytes
}

#[test]
#[cfg(feature = "vorbis")]
fn an_ogg_vorbis_at_a_rate_the_mixer_can_t_play_is_an_error() {
    let (audio, _mixer) = engine();
    // The checksum is made right: the same file at its own rate plays.
    audio
        .stream(Source::bytes(ogg_at(44_100)), Play::new())
        .unwrap();
    // A rate past what an engine runs at (whose stream's ring, sized by it,
    // would be as absurd) is refused, loaded whole or streamed.
    for rate in [10_000_000, 999] {
        assert!(
            audio.load(Source::bytes(ogg_at(rate))).is_err(),
            "{rate} Hz"
        );
        assert!(
            audio
                .stream(Source::bytes(ogg_at(rate)), Play::new())
                .is_err(),
            "{rate} Hz streamed"
        );
    }
}

#[test]
fn streams_keep_to_the_engine_s_memory_budget_and_give_it_back() {
    // A stereo stream at 48 kHz holds its ring (half a second) and its
    // window (32,768 frames): 448 KiB. A megabyte holds two, not three.
    let (audio, mut mixer) = Audio::offline(Config {
        limiter: None,
        stream_memory: 1 << 20,
        ..Config::default()
    })
    .unwrap();
    let bytes = wav::write(&vec![0.1; 48_000 * 2], 2, 48_000, wav::Encoding::Float32).unwrap();
    let wav = || Source::bytes(bytes.clone());
    let first = audio.stream(wav(), Play::new()).unwrap();
    audio.stream(wav(), Play::new()).unwrap();
    let held = audio.stats().stream_memory;
    assert_eq!(held, 2 * (24_000 * 2 + 32_768 * 2) * 4);
    let refused = audio.stream(wav(), Play::new()).unwrap_err();
    assert!(
        matches!(refused, zgx_audio::Error::StreamMemory { .. }),
        "{refused:?}"
    );
    // One stopped and given back: room again.
    first.stop().unwrap();
    mixer.frames(4_800);
    audio.update();
    assert_eq!(audio.stats().stream_memory, held / 2);
    audio.stream(wav(), Play::new()).unwrap();
}

#[cfg(all(feature = "aac", target_os = "macos"))]
mod aac {
    use super::*;

    // tone.m4a and tone.aac: 0.5 s at 44.1 kHz, 440 Hz left and 660 Hz
    // right at 1/16 of full scale, written as 16-bit PCM by a script of
    // ours, encoded with macOS's afconvert (`-d aac -b 96000`, as m4af and
    // as adts).
    const M4A: &[u8] = include_bytes!("assets/tone.m4a");
    const ADTS: &[u8] = include_bytes!("assets/tone.aac");

    /// How much of `samples` (one channel at `rate`) is a sine of `hz`:
    /// its amplitude, by least squares over whole windows.
    fn amplitude(samples: &[f32], hz: f64, rate: f64) -> f64 {
        let w = hz * std::f64::consts::TAU / rate;
        let (mut ys, mut yc) = (0f64, 0f64);
        for (i, &y) in samples.iter().enumerate() {
            let (sin, cos) = (w * i as f64).sin_cos();
            ys += f64::from(y) * sin;
            yc += f64::from(y) * cos;
        }
        2. * ys.hypot(yc) / samples.len() as f64
    }

    #[test]
    fn aac_loads_whole_gapless_and_sounds_like_its_source() {
        let (audio, _mixer) = engine();
        let sound = audio.load(Source::bytes(M4A)).unwrap();
        assert_eq!((sound.channels(), sound.rate()), (2, 44_100));
        // The MP4 says its priming and padding: exactly the source's frames.
        assert_eq!(sound.frames(), 22_050);
        let file = Source::file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/assets/tone.m4a"
        ));
        assert_eq!(audio.load(file).unwrap().frames(), 22_050);
        // Raw ADTS can't say: the priming stays, a little longer.
        let adts = audio.load(Source::bytes(ADTS)).unwrap();
        assert!(adts.frames() >= 22_050 && adts.frames() < 22_050 + 4_096);
    }

    #[test]
    fn an_aac_stream_sounds_as_it_does_loaded_whole_and_as_its_tone() {
        let whole = {
            let (audio, mut mixer) = engine();
            let sound = audio.load(Source::bytes(M4A)).unwrap();
            audio.play(&sound, Play::new()).unwrap();
            mixer.frames(30_000)
        };
        let (audio, mut mixer) = engine();
        audio.stream(Source::bytes(M4A), Play::new()).unwrap();
        let streamed = blocks(&mut mixer, 30_000);
        // AudioToolbox decodes, and its last bit or two depend on where its
        // own buffers fall in the heap: two decoders of one file, made at
        // different moments, can differ by up to about 5e-8 (seen: from
        // frame 389, with only a struct's size changed elsewhere). Those
        // buffers can't be aligned from here; the formats decoded in Rust
        // match exactly.
        assert_eq!(whole.len(), streamed.len());
        let most = whole
            .iter()
            .zip(&streamed)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(most <= 1e-6, "the stream differs by {most:e}");
        // Played at 48 kHz: each side its tone at 1/16 of full scale, within
        // a decibel, and the other side's barely.
        let side = |c: usize| -> Vec<f32> {
            whole.as_chunks::<2>().0[2_000..20_000]
                .iter()
                .map(|f| f[c])
                .collect()
        };
        let (left, right) = (side(0), side(1));
        let db = |a: f64| 20. * (a / (1. / 16.)).log10();
        assert!(db(amplitude(&left, 440., 48_000.)).abs() < 1., "left");
        assert!(db(amplitude(&right, 660., 48_000.)).abs() < 1., "right");
        assert!(
            db(amplitude(&left, 660., 48_000.)) < -40.,
            "right in the left"
        );
    }

    #[test]
    fn a_looping_aac_stream_comes_round_again() {
        // Half a second of tone, looped: its second and third times round
        // are as loud as its first.
        let (audio, mut mixer) = engine();
        audio
            .stream(Source::bytes(M4A), Play::new().looping(true))
            .unwrap();
        let out = blocks(&mut mixer, 72_000);
        let left: Vec<f32> = out.as_chunks::<2>().0.iter().map(|f| f[0]).collect();
        for second in [2_000..20_000, 50_000..68_000] {
            let a = amplitude(&left[second.clone()], 440., 48_000.);
            assert!((20. * (a * 16.).log10()).abs() < 1., "{second:?}: {a}");
        }
    }

    #[test]
    fn what_isn_t_aac_isn_t_taken_for_it() {
        let (audio, _mixer) = engine();
        let error = audio
            .load(Source::bytes(&b"not a sound at all"[..]))
            .unwrap_err();
        assert!(error.to_string().contains("AAC"), "{error}");
    }
}

/// How much of `samples` (one channel at `rate`) is a sine of `hz`: its
/// amplitude, by least squares over whole windows.
#[cfg(any(feature = "mp3", feature = "opus"))]
fn sine_amplitude(samples: &[f32], hz: f64, rate: f64) -> f64 {
    let w = hz * std::f64::consts::TAU / rate;
    let (mut ys, mut yc) = (0f64, 0f64);
    for (i, &y) in samples.iter().enumerate() {
        let (sin, cos) = (w * i as f64).sin_cos();
        ys += f64::from(y) * sin;
        yc += f64::from(y) * cos;
    }
    2. * ys.hypot(yc) / samples.len() as f64
}

/// A format's tone (0.5 s, 440 Hz left and 660 Hz right at 1/16 of full
/// scale) loaded whole: exactly `frames` at `rate`, its frames 4,000 to
/// 8,096 within `worst_db` of `reference`'s (another decoder's, of the
/// same file), and streamed and looped as loaded.
#[cfg(any(feature = "mp3", feature = "opus"))]
fn a_tone(bytes: &'static [u8], rate: u32, reference: &[u8], worst_db: f64) {
    let (audio, _mixer) = engine();
    let sound = audio.load(Source::bytes(bytes)).unwrap();
    assert_eq!((sound.channels(), sound.rate()), (2, rate));
    assert_eq!(sound.frames(), rate as usize / 2, "gapless");
    let reference = wav::read(reference).unwrap();
    let mine = {
        let (audio, mut mixer) = Audio::offline(Config {
            rate,
            limiter: None,
            ..Config::default()
        })
        .unwrap();
        audio.play(&sound, Play::new().fade_in(0.)).unwrap();
        mixer.frames(rate as usize / 2)
    };
    let worst = mine[4_000 * 2..8_096 * 2]
        .iter()
        .zip(&reference.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(
        20. * f64::from(worst).log10() < worst_db,
        "{:.1} dB from the reference",
        20. * f64::from(worst).log10()
    );
    // Each side its tone.
    let side = |c: usize| -> Vec<f32> {
        mine.as_chunks::<2>().0[2_000..20_000]
            .iter()
            .map(|f| f[c])
            .collect()
    };
    let db = |a: f64| 20. * (a * 16.).log10();
    assert!(db(sine_amplitude(&side(0), 440., f64::from(rate))).abs() < 1.);
    assert!(db(sine_amplitude(&side(1), 660., f64::from(rate))).abs() < 1.);
    // Streamed: the same samples. Looped: round again as loud.
    let whole = {
        let (audio, mut mixer) = engine();
        audio.play(&sound, Play::new()).unwrap();
        mixer.frames(30_000)
    };
    let (audio, mut mixer) = engine();
    audio.stream(Source::bytes(bytes), Play::new()).unwrap();
    assert_eq!(blocks(&mut mixer, 30_000), whole, "streamed");
    let (audio, mut mixer) = engine();
    audio
        .stream(Source::bytes(bytes), Play::new().looping(true))
        .unwrap();
    let looped = blocks(&mut mixer, 72_000);
    let left: Vec<f32> = looped.as_chunks::<2>().0[50_000..68_000]
        .iter()
        .map(|f| f[0])
        .collect();
    assert!(
        db(sine_amplitude(&left, 440., 48_000.)).abs() < 1.,
        "looped"
    );
}

#[cfg(feature = "mp3")]
#[test]
fn mp3_loads_gapless_as_the_platform_decodes_it_and_streams_and_loops() {
    // tone.mp3: our tone, at 44.1 kHz, encoded by rusty_mp3 at 128 kbps;
    // tone_mp3_audiotoolbox.wav: frames 4,000 to 8,096 of macOS's decode
    // of it (afconvert to 32-bit float). Two decoders: within -90 dB.
    a_tone(
        include_bytes!("assets/tone.mp3"),
        44_100,
        include_bytes!("assets/tone_mp3_audiotoolbox.wav"),
        -90.,
    );
}

#[cfg(feature = "opus")]
#[test]
fn opus_loads_gapless_as_another_decoder_does_and_streams_and_loops() {
    // tone.opus: our tone at 48 kHz, encoded by opus-pure at 96 kb/s and
    // written in Ogg with its pre-skip and end trim; tone_opus_opus1.wav:
    // frames 4,000 to 8,096 of opus1's decode of it (a separate port of
    // libopus), trimmed the same. Within -140 dB.
    a_tone(
        include_bytes!("assets/tone.opus"),
        48_000,
        include_bytes!("assets/tone_opus_opus1.wav"),
        -140.,
    );
}

#[test]
fn a_stream_started_partway_sounds_as_the_sound_started_there() {
    // 30,000 frames into a second of noise: the decoder drops what comes
    // before. Looping, 100,000 in is 4,000 round twice over.
    let bytes = noise(48_000, 2, 48_000);
    for (offset, looping) in [(30_000u64, false), (100_000, true)] {
        let whole = {
            let (audio, mut mixer) = engine();
            let sound = audio.load(Source::bytes(bytes.clone())).unwrap();
            audio
                .play(&sound, Play::new().offset(offset).looping(looping))
                .unwrap();
            mixer.frames(20_000)
        };
        let (audio, mut mixer) = engine();
        audio
            .stream(
                Source::bytes(bytes.clone()),
                Play::new().offset(offset).looping(looping),
            )
            .unwrap();
        let streamed = blocks(&mut mixer, 20_000);
        let at = whole.iter().zip(&streamed).position(|(a, b)| a != b);
        assert_eq!(at, None, "{offset} in: the stream differs at sample {at:?}");
        assert!(streamed.iter().any(|&s| s != 0.));
        assert_eq!(audio.stats().dry, 0);
    }
}
