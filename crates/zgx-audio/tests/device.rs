//! The system's output: a sound plays, the device closes after the silence
//! that follows (and pulls no more buffers), and the next sound opens it
//! again. The sound is silence itself (zeros), so nothing is heard. Where
//! there's no output (a CI runner without an audio device) it says so and
//! passes, unless `ZGX_AUDIO_REQUIRE_DEVICE` is set (CI's Linux job, whose
//! ALSA writes to a file).
use std::time::{Duration, Instant};
use zgx_audio::{Engine, Play};

fn until(mut what: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_device_plays_closes_after_silence_and_opens_again() {
    let mut engine = match Engine::open() {
        Ok(engine) => engine,
        Err(error) if std::env::var_os("ZGX_AUDIO_REQUIRE_DEVICE").is_none() => {
            println!("device: none here ({error}): skipped");
            return;
        }
        Err(error) => panic!("no output, and one is required: {error}"),
    };
    engine.close_after(0.3);
    let rate = engine.rate();
    let hush = engine.sound(&vec![0.; rate as usize / 5], rate);
    engine.play(&hush, Play::default());
    assert!(until(|| engine.is_running()));
    std::thread::sleep(Duration::from_millis(200));
    let played = engine.reading();
    assert_eq!(played.started, 1, "{played:?}");
    assert!(played.frames > 0);

    assert!(until(|| !engine.is_running()), "closed after its silence");
    let closed = engine.reading().frames;
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.reading().frames, closed, "closed: no buffers pulled");

    engine.play(&hush, Play::default());
    assert!(engine.is_running(), "a sound opens it");
    assert!(until(|| engine.reading().started == 2), "and it plays");
    assert!(engine.reading().frames > closed);
    println!(
        "device: {rate} Hz, played, closed after 0.3 s of silence, opened again for the next sound"
    );
}
