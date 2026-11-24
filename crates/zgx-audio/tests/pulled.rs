//! An engine the app pulls blocks from (`Audio::pulled`), as a browser's
//! audio worklet does: the same samples as an offline render, nothing
//! asked for while nothing sounds, and streams decoded inline, with no
//! thread.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use zgx_audio::{Audio, Config, Play, Pull, Ramp, Source, wav};

fn config() -> Config {
    Config {
        limiter: None,
        hold: Duration::from_millis(100),
        ..Config::default()
    }
}

/// A pulled engine, and how many times it has asked to be pulled.
fn pulled() -> (Audio, Pull, Arc<AtomicUsize>) {
    let woken = Arc::new(AtomicUsize::new(0));
    let count = woken.clone();
    let (audio, pull) = Audio::pulled(config(), move || {
        count.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    (audio, pull, woken)
}

fn noise(frames: usize, channels: usize, rate: u32) -> Vec<u8> {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
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

/// `frames` frames pulled in blocks of `block`, as a worklet asks.
fn pull_blocks(pull: &mut Pull, frames: usize, block: usize) -> Vec<f32> {
    let channels = pull.channels();
    let mut out = Vec::new();
    while out.len() < frames * channels {
        let n = block.min(frames - out.len() / channels);
        let mut buf = vec![0.; n * channels];
        pull.fill(&mut buf);
        out.extend(buf);
    }
    out
}

#[test]
fn a_pulled_engine_sounds_as_an_offline_one() {
    let bytes = noise(24_000, 2, 48_000);
    let offline = {
        let (audio, mut mixer) = Audio::offline(config()).unwrap();
        let sound = audio.load(Source::bytes(bytes.clone())).unwrap();
        let voice = audio.play(&sound, Play::new().gain(0.6)).unwrap();
        voice.set_pan(-0.3, Ramp::seconds(0.05)).unwrap();
        mixer.frames(30_000)
    };
    let (audio, mut pull, woken) = pulled();
    let sound = audio.load(Source::bytes(bytes)).unwrap();
    let voice = audio.play(&sound, Play::new().gain(0.6)).unwrap();
    voice.set_pan(-0.3, Ramp::seconds(0.05)).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 1, "a play asks to be pulled");
    for block in [128, 37, 300, 1, 113] {
        let (audio, mut again, _) = pulled();
        let sound = audio.load(Source::bytes(noise(24_000, 2, 48_000))).unwrap();
        let voice = audio.play(&sound, Play::new().gain(0.6)).unwrap();
        voice.set_pan(-0.3, Ramp::seconds(0.05)).unwrap();
        let blocks = pull_blocks(&mut again, 30_000, block);
        let at = offline.iter().zip(&blocks).position(|(a, b)| a != b);
        assert_eq!(at, None, "blocks of {block} differ at sample {at:?}");
    }
    let pulled = pull_blocks(&mut pull, 30_000, 128);
    assert!(pulled.iter().any(|&s| s != 0.));
}

/// Nothing is asked for before a sound, or once the quiet hold has passed:
/// an app pulling from it does no work while it's silent.
#[test]
fn a_pulled_engine_is_asked_for_nothing_while_silent() {
    let (audio, mut pull, woken) = pulled();
    // Commands that make no sound start nothing.
    audio
        .bus("music")
        .unwrap()
        .set_gain(0.5, Ramp::NONE)
        .unwrap();
    audio.set_ui_sounds(false).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 0);
    assert!(!pull.running());
    let mut block = vec![1.; 256];
    assert!(!pull.fill(&mut block), "asked while stopped, it says stop");
    assert!(block.iter().all(|&s| s == 0.), "and fills silence");
    assert_eq!(audio.stats().starts, 0);

    let samples = vec![0.5_f32; 480];
    let click = zgx_audio::Sound::from_samples(&samples, 1, 48_000).unwrap();
    audio.play(&click, Play::new()).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    assert!(pull.running());
    // A second play while it's pulled asks nothing more.
    audio.play(&click, Play::new()).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    // It goes on until the click and the hold (100 ms) have passed.
    let mut frames = 0;
    let mut block = vec![0.; 256];
    while pull.fill(&mut block) {
        frames += 128;
        assert!(frames < 48_000, "still asking after a second");
    }
    assert!(
        (4_800..=4_800 + 128 + 480).contains(&frames),
        "stopped after {frames} frames"
    );
    assert!(!pull.running());
    let stats = audio.stats();
    assert_eq!((stats.starts, stats.stops), (1, 1));
    assert!(!stats.running);
    // The next sound asks again.
    audio.play(&click, Play::new()).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 2);
    assert!(pull.running());
}

/// A pulled engine's stream is decoded as blocks are asked for, with no
/// decode thread, and sounds as the same source loaded whole.
#[test]
fn a_pulled_engines_stream_is_decoded_inline() {
    let bytes = noise(48_000, 2, 44_100);
    let whole = {
        let (audio, mut mixer) = Audio::offline(config()).unwrap();
        let sound = audio.load(Source::bytes(bytes.clone())).unwrap();
        audio.play(&sound, Play::new().gain(0.7)).unwrap();
        mixer.frames(60_000)
    };
    let (audio, mut pull, _) = pulled();
    let voice = audio
        .stream(Source::bytes(bytes), Play::new().gain(0.7))
        .unwrap();
    let streamed = pull_blocks(&mut pull, 60_000, 128);
    let at = whole.iter().zip(&streamed).position(|(a, b)| a != b);
    assert_eq!(at, None, "the stream differs at sample {at:?}");
    audio.update();
    assert!(!voice.playing());
}
