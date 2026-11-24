//! The pulled engine run on wasm32 itself (in Node, through
//! wasm-bindgen-test): a browser has no threads and no `Instant`, so
//! anything on this path that spawns one or reads the clock panics here,
//! where a native test or clippy wouldn't see it.
//! `cargo test -p zgx-audio --target wasm32-unknown-unknown --test web`
//! with wasm-bindgen-test-runner as the target's runner.
#![cfg(target_arch = "wasm32")]
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wasm_bindgen_test::wasm_bindgen_test;
use zgx_audio::{Audio, Config, Play, Source, wav};

#[wasm_bindgen_test]
fn a_pulled_engine_plays_and_streams_without_threads() {
    let woken = Arc::new(AtomicUsize::new(0));
    let count = woken.clone();
    let (audio, mut pull) = Audio::pulled(
        Config {
            hold: std::time::Duration::from_millis(50),
            ..Config::default()
        },
        move || {
            count.fetch_add(1, Ordering::SeqCst);
        },
    )
    .unwrap();
    pull.set_rate(44_100);
    let samples: Vec<f32> = (0..8_820)
        .map(|i| ((i as f32) * 0.05).sin() * 0.3)
        .collect();
    let bytes = wav::write(&samples, 1, 44_100, wav::Encoding::Float32).unwrap();
    let sound = audio.load(Source::bytes(bytes.clone())).unwrap();
    audio.play(&sound, Play::new()).unwrap();
    let voice = audio.stream(Source::bytes(bytes), Play::new()).unwrap();
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    let mut block = vec![0.; 256];
    let mut heard = false;
    let mut blocks = 0;
    while pull.fill(&mut block) {
        heard |= block.iter().any(|&s| s != 0.);
        blocks += 1;
        assert!(blocks < 1_000, "still asking");
    }
    assert!(heard);
    audio.update();
    assert!(!voice.playing());
    let stats = audio.stats();
    assert_eq!((stats.starts, stats.stops), (1, 1));
}
