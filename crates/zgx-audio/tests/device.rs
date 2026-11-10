//! The host that starts and stops a device, driven with a device of the
//! test's own: a thread that renders blocks until stopped, which reports
//! what it was asked to do.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zgx_audio::{Audio, Config, Device, Format, Play, Renderer, Sound, Time, Timing};

#[derive(Clone, Default)]
struct Controls {
    log: Arc<Mutex<Vec<String>>>,
    lose: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
    rate: Arc<AtomicU32>,
}

impl Controls {
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
    fn count(&self, what: &str) -> usize {
        self.log().iter().filter(|l| l.starts_with(what)).count()
    }
}

struct Fake {
    controls: Controls,
    running: Option<(JoinHandle<Box<Renderer>>, Arc<AtomicBool>)>,
}

impl Fake {
    fn new() -> (Fake, Controls) {
        let controls = Controls::default();
        controls.rate.store(48_000, Ordering::Relaxed);
        (
            Fake {
                controls: controls.clone(),
                running: None,
            },
            controls,
        )
    }
}

impl Fake {
    fn end(&mut self, how: &str) -> Option<Box<Renderer>> {
        let (thread, stop) = self.running.take()?;
        self.controls.log.lock().unwrap().push(how.into());
        stop.store(true, Ordering::Relaxed);
        Some(thread.join().unwrap())
    }
}

impl Device for Fake {
    fn format(&mut self) -> Result<Format, String> {
        Ok(Format {
            rate: self.controls.rate.load(Ordering::Relaxed),
            channels: 2,
        })
    }

    fn start(
        &mut self,
        mut renderer: Box<Renderer>,
        format: Format,
    ) -> Result<(), (String, Box<Renderer>)> {
        if self.controls.fail.swap(false, Ordering::Relaxed) {
            self.controls.log.lock().unwrap().push("failed".into());
            return Err(("the fake device is unplugged".into(), renderer));
        }
        self.controls
            .log
            .lock()
            .unwrap()
            .push(format!("start {}", format.rate));
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut out = vec![0f32; 256 * 2];
            let began = Instant::now();
            while !stopping.load(Ordering::Relaxed) {
                let at = began.elapsed().as_nanos() as u64;
                renderer.render(
                    &mut out,
                    Some(Timing {
                        asked: at,
                        heard: at + 5_000_000,
                    }),
                );
                // At the pace of a real device: 256 frames at the rate.
                std::thread::sleep(Duration::from_micros(
                    256 * 1_000_000 / u64::from(format.rate),
                ));
            }
            renderer
        });
        self.running = Some((thread, stop));
        Ok(())
    }

    fn stop(&mut self) -> Option<Box<Renderer>> {
        self.end("stop")
    }

    fn close(&mut self) -> Option<Box<Renderer>> {
        self.end("close")
    }

    fn lost(&mut self) -> bool {
        self.controls.lose.swap(false, Ordering::Relaxed)
    }
}

fn engine(hold: Duration) -> (Audio, Controls) {
    let (fake, controls) = Fake::new();
    (
        Audio::with_device(
            Config {
                hold,
                gain: 0.,
                ..Config::default()
            },
            fake,
        )
        .unwrap(),
        controls,
    )
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

fn blip() -> Sound {
    Sound::from_samples(&[0., 0.5, -0.25, 0.1], 1, 48_000).unwrap()
}

#[test]
fn the_device_starts_with_the_first_sound_and_stops_after_its_hold() {
    let (audio, controls) = engine(Duration::from_millis(100));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        controls.log(),
        Vec::<String>::new(),
        "nothing opened before a sound"
    );
    assert_eq!(audio.stats().starts, 0);
    let began = Instant::now();
    audio.play(&blip(), Play::new()).unwrap();
    until("a start", || audio.stats().running);
    until("a stop", || audio.stats().stops == 1);
    // The blip lasts four frames; the hold is the rest.
    assert!(
        began.elapsed() >= Duration::from_millis(100),
        "{:?}",
        began.elapsed()
    );
    assert_eq!(controls.log(), ["start 48000", "stop"]);
    assert!(audio.stats().sounding > 0);
    // And the next sound starts it again.
    audio.play(&blip(), Play::new()).unwrap();
    until("a second start", || audio.stats().starts == 2);
    until("a second stop", || audio.stats().stops == 2);
}

#[test]
fn a_sound_due_later_keeps_the_device_running_until_it_has_played() {
    let (audio, controls) = engine(Duration::from_millis(30));
    audio.play(&blip(), Play::new()).unwrap();
    until("a start", || audio.stats().running);
    // Due 300 ms from now: ten holds away.
    let at = audio.now() + audio.seconds(0.3);
    let voice = audio.play(&blip(), Play::new().at(at)).unwrap();
    until("it to play", || audio.now() > at + 10);
    assert_eq!(
        controls.count("stop"),
        0,
        "stopped before a sound due: {:?}",
        controls.log()
    );
    until("it to end", || !voice.playing());
    until("a stop", || audio.stats().stops == 1);
}

#[test]
fn prewarming_starts_the_device_without_a_sound_and_holds_it() {
    let (audio, controls) = engine(Duration::from_millis(80));
    let began = Instant::now();
    audio.prewarm();
    until("a start", || audio.stats().running);
    until("a stop", || audio.stats().stops == 1);
    assert!(began.elapsed() >= Duration::from_millis(80));
    assert_eq!(controls.log(), ["start 48000", "stop"]);
}

#[test]
fn a_lost_stream_starts_again_keeping_its_voices_and_the_new_rate() {
    let (audio, controls) = engine(Duration::from_millis(50));
    let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.1).sin() * 0.1).collect();
    let tone = Sound::from_samples(&tone, 1, 48_000).unwrap();
    let voice = audio.play(&tone, Play::new().looping(true)).unwrap();
    until("a start", || audio.stats().running);
    let before = audio.now();
    // The default output moves to a device at 44.1 kHz.
    controls.rate.store(44_100, Ordering::Relaxed);
    controls.lose.store(true, Ordering::Relaxed);
    until("a restart", || audio.stats().starts == 2);
    assert_eq!(audio.stats().losses, 1);
    assert_eq!(audio.rate(), 44_100);
    until("the clock to move on", || audio.now() > before + 4410);
    assert!(voice.playing(), "the voice kept playing across the change");
    assert_eq!(controls.log(), ["start 48000", "close", "start 44100"]);
    voice.stop().unwrap();
    until("a stop", || audio.stats().stops == 1);
}

#[test]
fn a_device_that_fails_to_start_says_why_and_is_tried_again_with_the_next_sound() {
    let (audio, controls) = engine(Duration::from_millis(30));
    controls.fail.store(true, Ordering::Relaxed);
    audio.play(&blip(), Play::new()).unwrap();
    until("a failure", || audio.stats().failures == 1);
    assert_eq!(
        audio.device_error().as_deref(),
        Some("the fake device is unplugged")
    );
    assert!(!audio.stats().running);
    audio.play(&blip(), Play::new().at(Time(0))).unwrap();
    until("a start", || audio.stats().starts == 1);
    until("a stop", || audio.stats().stops == 1);
    assert_eq!(controls.log(), ["failed", "start 48000", "stop"]);
}

#[test]
fn dropping_the_engine_closes_the_device() {
    let (audio, controls) = engine(Duration::from_secs(10));
    audio.play(&blip(), Play::new()).unwrap();
    until("a start", || audio.stats().running);
    drop(audio);
    // The host joined, closing the device on its way out.
    assert_eq!(controls.log(), ["start 48000", "close"]);
}

#[test]
fn a_device_that_reports_a_format_the_mixer_can_t_run_at_is_a_failure_not_a_panic() {
    for rate in [0, 10, 10_000_000] {
        let (audio, controls) = engine(Duration::from_millis(50));
        controls.rate.store(rate, Ordering::Relaxed);
        audio.play(&blip(), Play::new()).unwrap();
        until("a failure", || audio.stats().failures >= 1);
        assert_eq!(controls.count("start"), 0, "started at {rate} Hz");
        let why = audio.device_error().unwrap_or_default();
        assert!(why.contains(&rate.to_string()), "{rate} Hz: {why}");
        // A device that comes back right is used.
        controls.rate.store(48_000, Ordering::Relaxed);
        audio.play(&blip(), Play::new()).unwrap();
        until("a start", || audio.stats().running);
    }
}
