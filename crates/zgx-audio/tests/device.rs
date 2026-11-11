//! The host that starts and stops a device, driven with a device of the
//! test's own: a thread that renders blocks until stopped, which reports
//! what it was asked to do.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zgx_audio::{
    Audio, Config, Context, Device, Effect, Format, Play, Ramp, Renderer, Sound, Source, Time,
    Timing,
};

#[derive(Clone, Default)]
struct Controls {
    log: Arc<Mutex<Vec<String>>>,
    lose: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
    rate: Arc<AtomicU32>,
    /// Stops pause, keeping the renderer, rather than close.
    pause: Arc<AtomicBool>,
    /// Whether the thread that last started it has ended.
    host_ended: Arc<AtomicBool>,
    /// Times its blocks as WASAPI does: after each start and resume, a
    /// block stamped with a time left from before, then three that say
    /// 2 ms of output latency, then 42 ms, as it is.
    wasapi: Arc<AtomicBool>,
}

/// Set on the thread that starts the fake: says when that thread ends.
struct Ending(Arc<AtomicBool>);
impl Drop for Ending {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
thread_local! {
    static ENDING: std::cell::RefCell<Option<Ending>> = const { std::cell::RefCell::new(None) };
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
    /// A paused stream's renderer, and its format.
    paused: Option<(Box<Renderer>, Format)>,
}

impl Fake {
    fn new() -> (Fake, Controls) {
        let controls = Controls::default();
        controls.rate.store(48_000, Ordering::Relaxed);
        (
            Fake {
                controls: controls.clone(),
                running: None,
                paused: None,
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
        self.controls.log.lock().unwrap().push("format".into());
        Ok(Format {
            rate: self.controls.rate.load(Ordering::Relaxed),
            channels: 2,
        })
    }

    fn start(
        &mut self,
        renderer: Box<Renderer>,
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
        self.controls.host_ended.store(false, Ordering::Relaxed);
        let ended = self.controls.host_ended.clone();
        ENDING.with(|ending| *ending.borrow_mut() = Some(Ending(ended)));
        self.run(renderer, format);
        Ok(())
    }

    fn resume(&mut self) -> bool {
        let Some((renderer, format)) = self.paused.take() else {
            return false;
        };
        self.controls.log.lock().unwrap().push("resume".into());
        self.run(renderer, format);
        true
    }

    fn stop(&mut self) -> Option<Box<Renderer>> {
        if self.controls.pause.load(Ordering::Relaxed) {
            let format = Format {
                rate: self.controls.rate.load(Ordering::Relaxed),
                channels: 2,
            };
            let renderer = self.end("pause")?;
            self.paused = Some((renderer, format));
            return None;
        }
        self.end("stop")
    }

    fn close(&mut self) -> Option<Box<Renderer>> {
        if let Some((renderer, _)) = self.paused.take() {
            self.controls.log.lock().unwrap().push("close".into());
            return Some(renderer);
        }
        self.end("close")
    }

    fn lost(&mut self) -> bool {
        self.controls.lose.swap(false, Ordering::Relaxed)
    }
}

impl Fake {
    /// Renders on a thread of its own, at a device's pace.
    fn run(&mut self, mut renderer: Box<Renderer>, format: Format) {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let wasapi = self.controls.wasapi.load(Ordering::Relaxed);
        let thread = std::thread::spawn(move || {
            let mut out = vec![0f32; 256 * 2];
            let began = Instant::now();
            let mut block = 0;
            while !stopping.load(Ordering::Relaxed) {
                // A second on, so no time is 0.
                let at = 1_000_000_000 + began.elapsed().as_nanos() as u64;
                let timing = if !wasapi {
                    Timing {
                        asked: at,
                        heard: at + 5_000_000,
                    }
                } else if block == 0 {
                    Timing {
                        asked: 7,
                        heard: 7 + 2_000_000,
                    }
                } else {
                    let latency = if block <= 3 { 2_000_000 } else { 42_000_000 };
                    Timing {
                        asked: at,
                        heard: at + latency,
                    }
                };
                block += 1;
                renderer.render(&mut out, Some(timing));
                // At the pace of a real device: 256 frames at the rate.
                std::thread::sleep(Duration::from_micros(
                    256 * 1_000_000 / u64::from(format.rate),
                ));
            }
            renderer
        });
        self.running = Some((thread, stop));
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
    assert_eq!(controls.log(), ["format", "start 48000", "stop"]);
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
    assert_eq!(controls.log(), ["format", "start 48000", "stop"]);
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
    assert_eq!(
        controls.log(),
        ["format", "start 48000", "close", "format", "start 44100"]
    );
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
    assert_eq!(
        controls.log(),
        ["format", "failed", "format", "start 48000", "stop"]
    );
}

#[test]
fn a_play_that_starts_or_resumes_a_stream_is_placed_once_its_clock_settles() {
    // WASAPI's way: after a start and after a resume, a block with a time
    // left from before, then three saying 2 ms of output latency, then 42.
    // A play that starts the stream, or resumes it, is heard 42 ms after
    // its block is asked for, as every block after.
    let (fake, controls) = Fake::new();
    controls.wasapi.store(true, Ordering::Relaxed);
    controls.pause.store(true, Ordering::Relaxed);
    let audio = Audio::with_device(
        Config {
            hold: Duration::from_millis(50),
            limiter: None,
            gain: 0.,
            ..Config::default()
        },
        fake,
    )
    .unwrap();
    let latency = |audio: &Audio| {
        let stats = audio.stats();
        stats.play_heard as i64 - stats.play_asked as i64
    };
    audio.play(&blip(), Play::new()).unwrap();
    until("the play placed", || audio.stats().play_heard != 0);
    assert_eq!(latency(&audio), 42_000_000, "started");
    until("a pause", || controls.count("pause") == 1);
    let before = audio.stats().play_heard;
    audio.play(&blip(), Play::new()).unwrap();
    until("the play placed", || audio.stats().play_heard != before);
    assert_eq!(controls.count("resume"), 1);
    assert_eq!(latency(&audio), 42_000_000, "resumed");
}

#[test]
fn a_closed_stream_s_host_thread_ends_and_the_next_sound_starts_another() {
    let (audio, controls) = engine(Duration::from_millis(50));
    audio.play(&blip(), Play::new()).unwrap();
    until("a stop", || audio.stats().stops == 1);
    until("the host thread to end", || {
        controls.host_ended.load(Ordering::Relaxed)
    });
    audio.play(&blip(), Play::new()).unwrap();
    until("a second stop", || audio.stats().stops == 2);
    assert_eq!(
        controls.log(),
        [
            "format",
            "start 48000",
            "stop",
            "format",
            "start 48000",
            "stop"
        ]
    );
    until("the second host thread to end", || {
        controls.host_ended.load(Ordering::Relaxed)
    });
}

#[test]
fn a_paused_stream_is_closed_after_its_grace() {
    let (fake, controls) = Fake::new();
    controls.pause.store(true, Ordering::Relaxed);
    let audio = Audio::with_device(
        Config {
            hold: Duration::from_millis(50),
            close_after: Duration::from_millis(300),
            gain: 0.,
            ..Config::default()
        },
        fake,
    )
    .unwrap();
    audio.play(&blip(), Play::new()).unwrap();
    until("a pause", || controls.count("pause") == 1);
    let paused = Instant::now();
    // Played again within its grace: resumed, not started.
    audio.play(&blip(), Play::new()).unwrap();
    until("a second pause", || controls.count("pause") == 2);
    let again = Instant::now();
    until("a close", || controls.count("close") == 1);
    let grace = again.elapsed();
    assert!(
        grace >= Duration::from_millis(250),
        "closed {grace:?} after its pause"
    );
    assert!(paused.elapsed() < Duration::from_secs(5));
    assert_eq!(
        controls.log(),
        ["format", "start 48000", "pause", "resume", "pause", "close"]
    );
    until("the host thread to end", || {
        controls.host_ended.load(Ordering::Relaxed)
    });
}

#[test]
fn dropping_the_engine_closes_the_device() {
    let (audio, controls) = engine(Duration::from_secs(10));
    audio.play(&blip(), Play::new()).unwrap();
    until("a start", || audio.stats().running);
    drop(audio);
    // The host joined, closing the device on its way out.
    assert_eq!(controls.log(), ["format", "start 48000", "close"]);
}

#[test]
fn an_app_that_never_plays_a_sound_never_touches_the_device() {
    let (fake, controls) = Fake::new();
    let audio = Audio::with_device(
        Config {
            hold: Duration::from_millis(50),
            ..Config::default()
        },
        fake,
    )
    .unwrap();
    // Everything an app does at startup, short of a sound.
    let flat = Sound::from_samples(&[1.; 4800], 1, 48_000).unwrap();
    let ui = audio.bus("ui").unwrap();
    let effects = audio.add_bus("effects", &ui).unwrap();
    ui.set_gain(0.25, Ramp::NONE).unwrap();
    effects.mute(true).unwrap();
    effects.solo(false).unwrap();
    audio.bus("music").unwrap().drive().set(0.5);
    audio.set_ui_sounds(true).unwrap();
    assert!(audio.ui_sounds_enabled());
    let _wav = audio
        .load(Source::bytes(
            zgx_audio::wav::write(&[0.; 8], 1, 48_000, zgx_audio::wav::Encoding::Pcm24).unwrap(),
        ))
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        controls.log(),
        Vec::<String>::new(),
        "not even the format asked"
    );
    assert_eq!(audio.stats().starts, 0);
    // The first sound starts it, with what was set before already in place.
    audio.play(&flat, Play::new().bus(&ui)).unwrap();
    until("a start", || audio.stats().running);
    until("the sound", || audio.stats().peak > 0.);
    assert!(
        (audio.stats().peak - 0.25).abs() < 1e-6,
        "the gain set before: {}",
        audio.stats().peak
    );
    assert_eq!(controls.log()[..2], ["format", "start 48000"]);
}

#[test]
fn a_gain_set_while_the_device_is_stopped_does_not_start_it() {
    let (audio, controls) = engine(Duration::from_millis(40));
    audio.play(&blip(), Play::new()).unwrap();
    until("a stop", || audio.stats().stops == 1);
    audio
        .bus("music")
        .unwrap()
        .set_gain(0.3, Ramp::ms(10.))
        .unwrap();
    audio.master().mute(false).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(controls.log(), ["format", "start 48000", "stop"]);
    assert_eq!(audio.stats().starts, 1);
}

#[test]
fn the_app_is_told_when_voices_end() {
    let (audio, _controls) = engine(Duration::from_millis(500));
    let told = Arc::new(AtomicU32::new(0));
    let count = told.clone();
    audio.on_voices_ended(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let voice = audio.play(&blip(), Play::new()).unwrap();
    assert_eq!(audio.voices_playing(), 1);
    until("a notice", || told.load(Ordering::Relaxed) >= 1);
    assert_eq!(audio.take_ended(), [voice.id()]);
    assert_eq!(audio.voices_playing(), 0);
    assert_eq!(audio.take_ended(), Vec::<u64>::new());
}

/// An effect whose sound outlasts its input by as long as it holds (a
/// reverb's tail).
struct Ringing(Duration);
impl Effect for Ringing {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, _: &mut [f32], _: &Context<'_>) {}
    fn tail(&self) -> Duration {
        self.0
    }
}

#[test]
fn an_effect_s_tail_keeps_the_device_running_until_it_has_rung() {
    let (audio, controls) = engine(Duration::from_millis(100));
    let world = audio.bus("world").unwrap();
    world
        .add_effect(Ringing(Duration::from_millis(600)))
        .unwrap();
    let began = Instant::now();
    audio.play(&blip(), Play::new().bus(&world)).unwrap();
    until("a start", || audio.stats().running);
    until("a stop", || audio.stats().stops == 1);
    // Four frames of blip, 600 ms of tail, then the 100 ms hold.
    let took = began.elapsed();
    assert!(took >= Duration::from_millis(700), "stopped after {took:?}");
    assert_eq!(controls.log(), ["format", "start 48000", "stop"]);
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

/// A device whose first block comes untimed (asked at 0), as WASAPI's
/// does: it renders seven blocks at once as it starts, all but the first
/// timed.
struct ColdClock {
    held: Option<Box<Renderer>>,
    first: Option<Timing>,
}

/// The second block's time: a second on.
const COLD_SECOND: u64 = 1_000_000_000;
/// 256 frames at 48 kHz, in nanoseconds.
const COLD_BLOCK: u64 = 5_333_333;

impl Device for ColdClock {
    fn format(&mut self) -> Result<Format, String> {
        Ok(Format {
            rate: 48_000,
            channels: 2,
        })
    }
    fn start(
        &mut self,
        mut renderer: Box<Renderer>,
        _: Format,
    ) -> Result<(), (String, Box<Renderer>)> {
        let mut out = vec![0f32; 256 * 2];
        renderer.render(&mut out, self.first);
        // Then timed blocks on a steady clock, past the settling ones.
        for k in 0..6 {
            let asked = COLD_SECOND + k * COLD_BLOCK;
            renderer.render(
                &mut out,
                Some(Timing {
                    asked,
                    heard: asked + 10_000_000,
                }),
            );
        }
        self.held = Some(renderer);
        Ok(())
    }
    fn stop(&mut self) -> Option<Box<Renderer>> {
        self.held.take()
    }
    fn close(&mut self) -> Option<Box<Renderer>> {
        self.held.take()
    }
    fn lost(&mut self) -> bool {
        false
    }
}

#[test]
fn a_play_starting_an_untimed_first_block_is_timed_by_the_next() {
    // A play that starts the stream, its block asked at 0 (WASAPI's first,
    // filled before its clock runs): placed by the next block, it's heard
    // when it would be had the first block been timed a block earlier.
    let heard = |first: Option<Timing>| {
        let audio = Audio::with_device(
            Config {
                gain: 0.,
                ..Config::default()
            },
            ColdClock { held: None, first },
        )
        .unwrap();
        let sound = Sound::from_samples(&vec![0.25f32; 48_000], 1, 48_000).unwrap();
        audio.play(&sound, Play::new()).unwrap();
        until("a start", || audio.stats().running);
        let stats = audio.stats();
        (stats.play_asked, stats.play_heard)
    };
    let timed = heard(Some(Timing {
        asked: COLD_SECOND - COLD_BLOCK,
        heard: COLD_SECOND - COLD_BLOCK + 10_000_000,
    }));
    let cold = heard(Some(Timing {
        asked: 0,
        heard: 10_000_000,
    }));
    let untimed = heard(None);
    assert!(timed.1 > COLD_SECOND - COLD_BLOCK, "{timed:?}");
    for (name, got) in [("cold", cold), ("untimed", untimed)] {
        assert!(
            got.0.abs_diff(timed.0) <= 2 && got.1.abs_diff(timed.1) <= 2,
            "{name}: {got:?} against {timed:?}"
        );
    }
}

/// A device whose clock is at its end: it renders one block at once, as it
/// starts, timed there, and says whether that went through.
struct FarClock {
    held: Option<Box<Renderer>>,
    rendered: Arc<Mutex<Option<bool>>>,
}

impl Device for FarClock {
    fn format(&mut self) -> Result<Format, String> {
        Ok(Format {
            rate: 48_000,
            channels: 2,
        })
    }
    fn start(
        &mut self,
        mut renderer: Box<Renderer>,
        _: Format,
    ) -> Result<(), (String, Box<Renderer>)> {
        let mut out = vec![0f32; 256 * 2];
        let went = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.render(
                &mut out,
                Some(Timing {
                    asked: u64::MAX - 10,
                    heard: u64::MAX - 10,
                }),
            );
        }))
        .is_ok();
        *self.rendered.lock().unwrap() = Some(went);
        self.held = Some(renderer);
        Ok(())
    }
    fn stop(&mut self) -> Option<Box<Renderer>> {
        self.held.take()
    }
    fn close(&mut self) -> Option<Box<Renderer>> {
        self.held.take()
    }
    fn lost(&mut self) -> bool {
        false
    }
}

#[test]
fn a_device_clock_at_its_end_times_a_sound_without_overflowing() {
    let rendered = Arc::new(Mutex::new(None));
    let audio = Audio::with_device(
        Config {
            gain: 0.,
            ..Config::default()
        },
        FarClock {
            held: None,
            rendered: rendered.clone(),
        },
    )
    .unwrap();
    // Due a few frames in: started in the rendered block, which times it.
    audio.play(&blip(), Play::new().at(Time(10))).unwrap();
    until("a block", || rendered.lock().unwrap().is_some());
    assert_eq!(*rendered.lock().unwrap(), Some(true), "the render panicked");
}

/// A device whose stream won't close: a driver that hangs.
struct Stuck(Option<Box<Renderer>>);

impl Device for Stuck {
    fn format(&mut self) -> Result<Format, String> {
        Ok(Format {
            rate: 48_000,
            channels: 2,
        })
    }
    fn start(&mut self, renderer: Box<Renderer>, _: Format) -> Result<(), (String, Box<Renderer>)> {
        self.0 = Some(renderer);
        Ok(())
    }
    fn stop(&mut self) -> Option<Box<Renderer>> {
        self.close()
    }
    fn close(&mut self) -> Option<Box<Renderer>> {
        std::thread::sleep(Duration::from_secs(20));
        self.0.take()
    }
    fn lost(&mut self) -> bool {
        false
    }
}

#[test]
fn dropping_the_engine_doesn_t_wait_on_a_device_that_won_t_close() {
    let audio = Audio::with_device(
        Config {
            gain: 0.,
            hold: Duration::from_secs(60),
            ..Config::default()
        },
        Stuck(None),
    )
    .unwrap();
    audio.prewarm();
    until("a start", || audio.stats().running);
    let began = Instant::now();
    drop(audio);
    let took = began.elapsed();
    assert!(took < Duration::from_secs(5), "dropping waited {took:?}");
}
