//! The audio thread's rules, checked: while the mixer renders, nothing on
//! this thread allocates, frees or reallocates (a counting global
//! allocator), and nothing waits or calls the system (on macOS the task's
//! system calls and context switches, counted by the kernel; on Linux this
//! thread's voluntary context switches). Every path a voice or a bus can
//! take is driven through it: plays at each rate and channel count, both
//! interpolators, every ramp and curve, starts, stops, steals, drops, loops,
//! the limiter, a return queue left full, outputs of 1, 2 and 6 channels.
//!
//! A binary of its own without the test harness: no other thread runs but
//! the decode thread, which sleeps through every armed stretch, so the
//! kernel's counts for the task are this thread's. Commands are sent,
//! timed for later frames, before each armed render: sending is the main
//! thread's work and may allocate; applying them is the mixer's and mustn't.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use zgx_audio::{Audio, Config, Curve, Interpolation, Offline, Play, Ramp, Sound, Time};

struct Counting;

thread_local! {
    /// Set on the thread under test alone: the decode thread's own
    /// allocations aren't the mixer's.
    static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call goes to the system allocator unchanged; the counters
// are atomics, which allocate nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.get() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ARMED.get() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.get() {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ARMED.get() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            FREES.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// What the kernel counts that a wait or a system call moves: on macOS
/// the task's system calls (every wait is one; its context switch count
/// includes being preempted, which isn't the mixer's doing, so it isn't
/// used), on Linux this thread's voluntary context switches.
#[cfg(target_os = "macos")]
fn kernel() -> (i64, i64) {
    #[repr(C)]
    #[derive(Default)]
    struct TaskEventsInfo {
        faults: i32,
        pageins: i32,
        cow_faults: i32,
        messages_sent: i32,
        messages_received: i32,
        syscalls_mach: i32,
        syscalls_unix: i32,
        csw: i32,
    }
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: i32, info: *mut TaskEventsInfo, count: *mut u32) -> i32;
    }
    let mut info = TaskEventsInfo::default();
    let mut count = 8u32;
    // SAFETY: TASK_EVENTS_INFO (2) fills eight integers.
    let ok = unsafe { task_info(mach_task_self_, 2, &mut info, &mut count) } == 0;
    assert!(ok, "task_info");
    (
        i64::from(info.syscalls_unix) + i64::from(info.syscalls_mach),
        0,
    )
}

#[cfg(target_os = "linux")]
fn kernel() -> (i64, i64) {
    #[repr(C)]
    struct Rusage {
        times: [i64; 4],
        counts: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = Rusage {
        times: [0; 4],
        counts: [0; 14],
    };
    // SAFETY: RUSAGE_THREAD (1) fills a struct rusage: two timevals, then
    // fourteen longs, of which ru_nvcsw is the thirteenth.
    assert_eq!(unsafe { getrusage(1, &mut usage) }, 0);
    (0, usage.counts[12])
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn kernel() -> (i64, i64) {
    (0, 0)
}

/// What one armed stretch did.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
struct Broke {
    allocs: u64,
    frees: u64,
    calls: i64,
    switches: i64,
}

/// What reading the counters costs, calibrated once: an empty armed stretch.
fn armed(work: impl FnOnce()) -> Broke {
    let (allocs, frees) = (
        ALLOCS.load(Ordering::Relaxed),
        FREES.load(Ordering::Relaxed),
    );
    let (calls, switches) = kernel();
    ARMED.set(true);
    work();
    ARMED.set(false);
    let (calls_after, switches_after) = kernel();
    Broke {
        allocs: ALLOCS.load(Ordering::Relaxed) - allocs,
        frees: FREES.load(Ordering::Relaxed) - frees,
        calls: calls_after - calls,
        switches: switches_after - switches,
    }
}

struct Run {
    name: &'static str,
    audio: Audio,
    mixer: Offline,
    out: Vec<f32>,
    /// What an empty stretch costs the kernel's counters.
    floor: Broke,
    heard: bool,
    worst: Broke,
}

impl Run {
    fn new(name: &'static str, config: Config, floor: Broke) -> Run {
        let channels = config.channels;
        let (audio, mixer) = Audio::offline(config);
        Run {
            name,
            audio,
            mixer,
            out: vec![0.; 256 * channels],
            floor,
            heard: false,
            worst: Broke::default(),
        }
    }

    /// Renders `blocks` blocks of 256 frames armed, one stretch each.
    fn render(&mut self, blocks: usize) {
        for _ in 0..blocks {
            // Streams are filled on the main thread's side, then the mixer
            // runs as a device runs it.
            self.audio.settle();
            let (mixer, out) = (&mut self.mixer, &mut self.out);
            let broke = armed(|| mixer.render_now(out));
            self.heard |= self.out.iter().any(|&s| s != 0.);
            let over = Broke {
                allocs: broke.allocs,
                frees: broke.frees,
                calls: broke.calls - self.floor.calls,
                switches: broke.switches - self.floor.switches,
            };
            self.worst.allocs = self.worst.allocs.max(over.allocs);
            self.worst.frees = self.worst.frees.max(over.frees);
            self.worst.calls = self.worst.calls.max(over.calls);
            self.worst.switches = self.worst.switches.max(over.switches);
        }
    }

    fn at(&self, seconds: f64) -> Time {
        self.audio.now() + self.audio.seconds(seconds)
    }

    fn finish(self, failures: &mut Vec<String>) {
        let stats = self.audio.stats();
        println!(
            "{:28} {:>7} frames, {} blocks, {} voices at the end, {:?}",
            self.name, stats.frames, stats.blocks, stats.voices, self.worst
        );
        if !self.heard {
            failures.push(format!(
                "{}: made no sound, so it proved nothing",
                self.name
            ));
        }
        if self.worst != Broke::default() {
            failures.push(format!(
                "{}: the mixer broke the rules: {:?}",
                self.name, self.worst
            ));
        }
        if let Some(failure) = failures.last().filter(|f| f.starts_with(self.name)) {
            eprintln!("{failure}");
        }
    }
}

fn noise(frames: usize, channels: usize, seed: u64) -> Vec<f32> {
    let mut seed = seed | 1;
    let mut samples: Vec<f32> = (0..frames * channels)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    // Starting at zero gets no fade in; half of them start elsewhere.
    if seed.is_multiple_of(2) {
        samples[..channels].fill(0.);
    }
    samples
}

fn sounds() -> Vec<Sound> {
    let mut sounds = Vec::new();
    for (rate, channels) in [
        (48_000, 2),
        (48_000, 1),
        (44_100, 2),
        (44_100, 1),
        (22_050, 2),
        (96_000, 1),
    ] {
        sounds.push(
            Sound::from_samples(
                &noise(rate as usize / 10, channels, rate as u64),
                channels,
                rate,
            )
            .unwrap(),
        );
    }
    sounds
}

fn main() {
    let mut failures = Vec::new();
    // The counters' own cost, least of a few empty stretches.
    let floor = (0..20)
        .map(|_| armed(|| {}))
        .min_by_key(|b| (b.calls, b.switches))
        .unwrap();
    println!("an empty stretch: {floor:?}");
    assert_eq!((floor.allocs, floor.frees), (0, 0));

    // The checks catch what they're for.
    let caught = armed(|| drop(std::hint::black_box(vec![1u8; 64])));
    assert!(
        caught.allocs >= 1 && caught.frees >= 1,
        "the allocator counts: {caught:?}"
    );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let caught = armed(|| std::thread::sleep(std::time::Duration::from_millis(2)));
        assert!(
            caught.calls > floor.calls || caught.switches > floor.switches,
            "the kernel counts a sleep: {caught:?}"
        );
        let lock = std::sync::Arc::new(std::sync::Mutex::new(0));
        let held = lock.lock().unwrap();
        let other = {
            let lock = lock.clone();
            std::thread::spawn(move || *lock.lock().unwrap() += 1)
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(held);
        let caught = armed(|| other.join().unwrap());
        assert!(
            caught.calls > floor.calls || caught.switches > floor.switches,
            "the kernel counts waiting on a thread: {caught:?}"
        );
    }

    let sounds = sounds();
    for interpolation in [Interpolation::Cubic, Interpolation::Linear] {
        let name = if interpolation == Interpolation::Cubic {
            "every voice path, cubic"
        } else {
            "every voice path, linear"
        };
        let mut run = Run::new(
            name,
            Config {
                interpolation,
                ..Config::default()
            },
            floor,
        );
        let buses = ["ui", "music", "world"].map(|b| run.audio.bus(b).unwrap());
        let mut voices = Vec::new();
        for (i, sound) in sounds.iter().enumerate() {
            let at = run.at(0.001 * i as f64);
            let play = Play::new()
                .bus(&buses[i % 3])
                .gain(0.3)
                .pan(i as f32 / 3. - 1.)
                .at(at);
            voices.push(
                run.audio
                    .play(
                        sound,
                        if i % 2 == 0 {
                            play.looping(true)
                        } else {
                            play.fade_in(0.01)
                        },
                    )
                    .unwrap(),
            );
        }
        run.render(8);
        // Every ramp and curve on every voice, timed inside the next blocks.
        let curves = [Curve::Linear, Curve::EqualPower, Curve::Exponential];
        for (i, voice) in voices.iter().enumerate() {
            let at = run.at(0.002);
            let ramp = Ramp::ms(30.).curve(curves[i % 3]);
            voice.at(at).set_gain(0.1, ramp).unwrap();
            voice.at(at).set_pan(1. - i as f32 / 3., ramp).unwrap();
            voice.at(at).set_pitch(1.5 - i as f32 * 0.1, ramp).unwrap();
        }
        for (i, bus) in buses.iter().enumerate() {
            bus.at(run.at(0.003)).fade_to(0.5, 0.05, curves[i]).unwrap();
        }
        buses[1].at(run.at(0.004)).solo(true).unwrap();
        buses[2].at(run.at(0.005)).mute(true).unwrap();
        run.render(12);
        let extra = run.audio.add_bus("extra", &buses[0]).unwrap();
        buses[1].solo(false).unwrap();
        buses[2].mute(false).unwrap();
        run.audio
            .play(&sounds[2], Play::new().bus(&extra).loop_frames(100..900))
            .unwrap();
        for voice in voices.iter().step_by(2) {
            voice
                .at(run.at(0.004))
                .stop_after(Ramp::ms(20.).curve(Curve::EqualPower))
                .unwrap();
        }
        // A voice stopped before it starts, and a play already late.
        let early = run
            .audio
            .play(&sounds[0], Play::new().at(run.at(0.5)))
            .unwrap();
        early.stop().unwrap();
        run.audio.play(&sounds[1], Play::new().at(Time(0))).unwrap();
        run.render(30);
        run.finish(&mut failures);
    }

    // Sounds no one else holds: if the mixer dropped one, it would free it.
    let mut run = Run::new(
        "sounds only the mixer holds",
        Config {
            voices: 3,
            ..Config::default()
        },
        floor,
    );
    for i in 0..12 {
        let sound =
            Sound::from_samples(&noise(300 + i * 50, 1 + i % 2, i as u64), 1 + i % 2, 48_000)
                .unwrap();
        let voice = run
            .audio
            .play(&sound, Play::new().gain(0.05).at(run.at(0.001 * i as f64)))
            .unwrap();
        if i % 4 == 0 {
            voice.at(run.at(0.004)).stop().unwrap();
        }
    }
    run.render(12);
    run.finish(&mut failures);

    // Streams: the mixer pulls each from its ring, fades as it runs low,
    // runs dry and comes back; the decode thread fills between blocks
    // (its work, not the mixer's, so outside the armed stretch).
    let mut run = Run::new("streams", Config::default(), floor);
    for (i, (rate, channels)) in [(44_100, 2), (48_000, 1), (22_050, 2)]
        .into_iter()
        .enumerate()
    {
        let samples = noise(rate as usize, channels, i as u64 + 7);
        let bytes =
            zgx_audio::wav::write(&samples, channels, rate, zgx_audio::wav::Encoding::Float32);
        let play = Play::new()
            .gain(0.2)
            .looping(i != 1)
            .pitch(1. + i as f32 * 0.1);
        run.audio
            .stream(zgx_audio::Source::bytes(bytes), play)
            .unwrap();
    }
    for block in 0..60 {
        // Settled every other block: between, the rings run down, so
        // streams also run low, fade and come back.
        if block % 2 == 0 || block > 40 {
            run.audio.settle();
            // The decode thread, done, goes back to sleep: its calls are
            // the task's too, so it has to be asleep before the stretch.
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let (mixer, out) = (&mut run.mixer, &mut run.out);
        let broke = armed(|| mixer.render_now(out));
        run.heard |= out.iter().any(|&s| s != 0.);
        run.worst.allocs = run.worst.allocs.max(broke.allocs);
        run.worst.frees = run.worst.frees.max(broke.frees);
        run.worst.calls = run.worst.calls.max(broke.calls - floor.calls);
        run.worst.switches = run.worst.switches.max(broke.switches - floor.switches);
    }
    run.finish(&mut failures);

    // Too many voices: steals, and plays that lose and are dropped.
    let mut run = Run::new(
        "steals and drops",
        Config {
            voices: 4,
            ..Config::default()
        },
        floor,
    );
    for i in 0..40 {
        let play = Play::new()
            .gain(if i % 3 == 0 { 0.01 } else { 0.4 })
            .priority((i % 4) as i8)
            .looping(i % 5 == 0);
        run.audio
            .play(
                &sounds[i % sounds.len()],
                play.at(run.at(0.0005 * i as f64)),
            )
            .unwrap();
    }
    run.render(20);
    assert!(
        run.audio.stats().stolen > 0 && run.audio.stats().dropped > 0,
        "{:?}",
        run.audio.stats()
    );
    run.finish(&mut failures);

    // Loud enough for the limiter to work, on 1 and 6 channels.
    for channels in [1, 6] {
        let mut run = Run::new(
            if channels == 1 {
                "limiting, mono out"
            } else {
                "limiting, six channels out"
            },
            Config {
                channels,
                ..Config::default()
            },
            floor,
        );
        for sound in &sounds {
            run.audio
                .play(sound, Play::new().gain(4.).looping(true))
                .unwrap();
        }
        run.render(20);
        run.finish(&mut failures);
    }

    // Nobody drops what the mixer gives back: the return queue fills, and
    // ended voices keep their sounds (and slots) until there's room.
    let mut run = Run::new(
        "a full return queue",
        Config {
            voices: 2,
            ..Config::default()
        },
        floor,
    );
    let blip = Sound::from_samples(&[0., 0.5, -0.25, 0.1], 1, 48_000).unwrap();
    for i in 0..200 {
        run.audio
            .play(&blip, Play::new().at(Time(10 + i * 3)))
            .unwrap();
    }
    // Rendered without a call on `audio` between blocks: nothing drains it.
    let mut last = 0;
    for block in 0..10 {
        let (mixer, out) = (&mut run.mixer, &mut run.out);
        let broke = armed(|| mixer.render_now(out));
        if let Some(at) = out.iter().rposition(|&s| s != 0.) {
            last = block * 256 + at;
        }
        run.worst.allocs = run.worst.allocs.max(broke.allocs);
        run.worst.frees = run.worst.frees.max(broke.frees);
        run.worst.calls = run.worst.calls.max(broke.calls - floor.calls);
        run.worst.switches = run.worst.switches.max(broke.switches - floor.switches);
    }
    // The last blip is due at frame 607 and sounds to 610. The return queue
    // (128 places) and then every slot filled with sounds waiting to go
    // back, so the blips stopped before it...
    assert!(
        last > 0 && last < 600,
        "the return queue never filled: last sound at frame {last}"
    );
    // ...and once the main thread takes them back, the waiting plays start.
    run.audio.update();
    run.render(4);
    run.finish(&mut failures);

    if failures.is_empty() {
        println!("the mixer kept the rules");
    } else {
        eprintln!("{} scene(s) failed", failures.len());
        std::process::exit(1);
    }
}
