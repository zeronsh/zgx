//! Sounds scheduled on the audio clock reach the device on their exact
//! samples: three clicks a fifth of a second apart land 9,600 frames apart
//! at 48 kHz in what the device was given, however their commands were
//! spaced. Needs an output that writes what it's given to a file, named by
//! `ZGX_AUDIO_CAPTURE` (CI's Linux job: ALSA's file plugin); without one it
//! says so and passes, so nothing is played aloud.
use std::time::{Duration, Instant};
use zgx_audio::{Engine, Play};

#[test]
fn scheduled_clicks_land_on_their_samples() {
    let Some(capture) = std::env::var_os("ZGX_AUDIO_CAPTURE") else {
        println!("capture: no ZGX_AUDIO_CAPTURE: skipped");
        return;
    };
    let mut engine = Engine::open().expect("an output writing to ZGX_AUDIO_CAPTURE");
    engine.close_after(30.);
    let rate = engine.rate();
    let click = engine.sound(&[1., 1.], rate);
    // Far enough ahead that even an output that doesn't wait hasn't reached
    // it when the last arrives: a null device writes some 20 million frames
    // a second, so 20 s of sound is about 50 ms (and on a device that waits,
    // 20 s).
    let t = engine.now() + 20.;
    for (i, pause) in [0, 3, 11].into_iter().enumerate() {
        std::thread::sleep(Duration::from_micros(pause));
        let at = Some(t + i as f64 * 0.2);
        engine.play(
            &click,
            Play {
                at,
                ..Play::default()
            },
        );
    }
    let start = Instant::now();
    while engine.reading().started < 3 || engine.now() < t + 0.5 {
        assert!(start.elapsed() < Duration::from_secs(40), "it played");
        std::thread::sleep(Duration::from_millis(5));
    }
    let late = engine.reading().late;
    drop(engine);
    let bytes = std::fs::read(&capture).unwrap();
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let heard: Vec<u64> = samples
        .chunks_exact(2)
        .enumerate()
        .filter(|(_, frame)| frame[0] != 0.)
        .map(|(i, _)| i as u64)
        .collect();
    let first = (t * f64::from(rate)).round() as u64;
    let fifth = u64::from(rate) / 5;
    println!("capture: clicks at frames {heard:?}, {late} late");
    assert_eq!(late, 0);
    assert_eq!(heard, [first, first + fifth, first + 2 * fifth]);
}
