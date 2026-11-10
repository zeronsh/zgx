//! What the device costs and how fast it starts, on macOS, at zero gain:
//! master's gain is 0 from the first frame, so streams run and counters
//! count but nothing is heard. Every mode checks the output stayed silent.
//!
//! `cargo run --release -p zgx-audio --features device --example device_probe -- <mode>`
//!
//! - `none <s>`: the process's wakeups over `s` seconds, no audio at all
//! - `idle <s>`: the same with an engine made and a sound loaded, nothing played
//! - `after <s>`: one click played, its stream stopped after the hold, then `s` seconds
//! - `latency <n> <pause|close>`: `n` cold starts and `3n` plays with the stream running, play to heard
//! - `load <s>`: 64 voices looping for `s` seconds: block times, underruns
#[cfg(target_os = "macos")]
mod probe {
    use std::time::{Duration, Instant};
    use zgx_audio::{Audio, Config, Cpal, Play, Sound, Stopping};

    #[repr(C)]
    struct RusageInfoV4 {
        uuid: [u8; 16],
        fields: [u64; 40],
    }

    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV4) -> i32;
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, size: i32) -> i32;
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }

    /// Package-idle and interrupt wakeups of this process so far.
    fn wakeups() -> u64 {
        let mut info = RusageInfoV4 {
            uuid: [0; 16],
            fields: [0; 40],
        };
        // SAFETY: RUSAGE_INFO_V4 (4) fills a rusage_info_v4; fields 2 and 3
        // are ri_pkg_idle_wkups and ri_interrupt_wkups.
        assert_eq!(
            unsafe { proc_pid_rusage(std::process::id() as i32, 4, &mut info) },
            0
        );
        info.fields[2] + info.fields[3]
    }

    /// This process's threads.
    fn threads() -> i32 {
        let mut info = [0u8; 96];
        // SAFETY: PROC_PIDTASKINFO (4) fills a proc_taskinfo: six u64s, then
        // i32s, of which pti_threadnum is the tenth.
        let size = unsafe { proc_pidinfo(std::process::id() as i32, 4, 0, info.as_mut_ptr(), 96) };
        assert_eq!(size, 96);
        i32::from_ne_bytes(info[84..88].try_into().unwrap())
    }

    /// The clock CoreAudio's host times count, in nanoseconds
    /// (CLOCK_UPTIME_RAW: mach_absolute_time).
    fn clock() -> u64 {
        // SAFETY: a clock id the system defines.
        unsafe { clock_gettime_nsec_np(8) }
    }

    fn engine(stopping: Stopping) -> Audio {
        let config = Config {
            gain: 0.,
            ..Config::default()
        };
        assert_eq!(config.gain, 0., "every automated run is silent");
        Audio::with_device(config, Cpal::new().stopping(stopping)).expect("a config it runs with")
    }

    fn click() -> Sound {
        let samples: Vec<f32> = (0..480)
            .map(|i| {
                if i == 0 {
                    0.
                } else {
                    (i as f32 * 0.3).sin() * (1. - i as f32 / 480.) * 0.5
                }
            })
            .collect();
        Sound::from_samples(&samples, 1, 48_000).unwrap()
    }

    fn silent(audio: &Audio) {
        let stats = audio.stats();
        assert_eq!(stats.peak, 0., "the output stayed silent: {stats:?}");
        assert!(audio.device_error().is_none(), "{:?}", audio.device_error());
    }

    fn wait(what: &str, limit: Duration, done: impl Fn() -> bool) {
        let began = Instant::now();
        while !done() {
            assert!(began.elapsed() < limit, "waited {limit:?} for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn measure(seconds: u64) -> (u64, i32) {
        let before = wakeups();
        std::thread::sleep(Duration::from_secs(seconds));
        (wakeups() - before, threads())
    }

    /// Plays one click and returns how long from the call until the device
    /// asked for the block that starts it, and until its first frame is heard
    /// as the device's clock says, in microseconds.
    fn heard(audio: &Audio, click: &Sound) -> (f64, f64, f64) {
        let previous = audio.stats().play_heard;
        let called = clock();
        audio
            .play(click, Play::new().bus(&audio.bus("ui").unwrap()))
            .unwrap();
        wait("the click to be placed", Duration::from_secs(5), || {
            audio.stats().play_heard != previous
        });
        let stats = audio.stats();
        let since = |t: u64| (t as f64 - called as f64) / 1e3;
        (
            since(stats.play_mixed),
            since(stats.play_asked),
            since(stats.play_heard),
        )
    }

    fn spread(name: &str, mut values: Vec<f64>) {
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        println!(
            "{name}: min {:.0} median {:.0} max {:.0} µs ({} runs)",
            at(0.),
            at(0.5),
            at(1.),
            values.len()
        );
    }

    pub fn run() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mode = args.first().map(String::as_str).unwrap_or("idle");
        let number =
            |i: usize, default: u64| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
        let stopping = match args.get(2).map(String::as_str) {
            Some("pause") => Stopping::Pause,
            _ => Stopping::Close,
        };
        println!("threads at start: {}", threads());
        match mode {
            "none" => {
                let (wakes, threads) = measure(number(1, 60));
                println!("no audio: {wakes} wakeups, {threads} threads");
            }
            "idle" => {
                let audio = engine(stopping);
                let _sound = click();
                let (wakes, threads) = measure(number(1, 60));
                println!("engine made, nothing played: {wakes} wakeups, {threads} threads");
                assert_eq!(audio.stats().starts, 0);
            }
            "after" => {
                let audio = engine(stopping);
                let sound = click();
                audio.play(&sound, Play::new()).unwrap();
                wait("the stream to start", Duration::from_secs(5), || {
                    audio.stats().running
                });
                let running = threads();
                wait(
                    "the stream to stop after its hold",
                    Duration::from_secs(5),
                    || audio.stats().stops == 1,
                );
                let stopped = threads();
                let (wakes, threads) = measure(number(1, 60));
                let stats = audio.stats();
                println!(
                    "threads: {running} running, {stopped} just stopped, {threads} at the end"
                );
                println!("one click, then stopped ({stopping:?}): {wakes} wakeups; {stats:?}");
                assert!(stats.sounding > 0, "the click was mixed");
                silent(&audio);
            }
            "latency" => {
                let audio = engine(stopping);
                let sound = click();
                let runs = number(1, 10) as usize;
                let (mut cold, mut warm, mut starts) = (Vec::new(), Vec::new(), Vec::new());
                let (mut cold_asked, mut warm_asked) = (Vec::new(), Vec::new());
                let (mut cold_mixed, mut warm_mixed) = (Vec::new(), Vec::new());
                for _ in 0..runs {
                    wait("a stop", Duration::from_secs(5), || !audio.stats().running);
                    let (mixed, asked, heard) = heard(&audio, &sound);
                    cold_mixed.push(mixed);
                    cold_asked.push(asked);
                    cold.push(heard);
                    starts.push(audio.stats().start_micros as f64);
                }
                // Warm: the stream running, each play at another point of the
                // device's cycle (a play just after a block waits a whole one).
                audio.prewarm();
                wait("a start", Duration::from_secs(5), || audio.stats().running);
                for i in 0..runs * 3 {
                    std::thread::sleep(Duration::from_micros(1000 + (i as u64 * 3_731) % 11_000));
                    let (mixed, asked, heard) = heard(&audio, &sound);
                    warm_mixed.push(mixed);
                    warm_asked.push(asked);
                    warm.push(heard);
                }
                let stats = audio.stats();
                println!("{stopping:?}: {stats:?}");
                spread("cold, play to its block mixed (host clock)", cold_mixed);
                spread("cold, play to its block's timestamp", cold_asked);
                spread("cold, play to heard", cold);
                spread("warm, play to its block mixed (host clock)", warm_mixed);
                spread("warm, play to its block's timestamp", warm_asked);
                spread("warm, play to heard", warm);
                spread("the device's start", starts);
                println!(
                    "output latency (heard - asked) of the last block: {:.0} µs",
                    (stats.block_heard - stats.block_asked) as f64 / 1e3
                );
                assert!(stats.sounding > 0);
                silent(&audio);
            }
            "load" => {
                let audio = engine(stopping);
                let seconds = number(1, 10);
                let mut seed = 1u64;
                for i in 0..64 {
                    let noise: Vec<f32> = (0..96_000)
                        .map(|_| {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                        })
                        .collect();
                    let sound =
                        Sound::from_samples(&noise, 2, if i % 2 == 0 { 48_000 } else { 44_100 })
                            .unwrap();
                    audio
                        .play(&sound, Play::new().gain(0.01).looping(true))
                        .unwrap();
                }
                std::thread::sleep(Duration::from_secs(seconds));
                let stats = audio.stats();
                let times = audio.block_times();
                let total: u64 = times.iter().map(|&t| u64::from(t)).sum();
                let at = |q: f64| {
                    let mut seen = 0;
                    for (micros, &count) in times.iter().enumerate() {
                        seen += u64::from(count);
                        if seen as f64 >= total as f64 * q {
                            return micros;
                        }
                    }
                    times.len()
                };
                let period = 256. / f64::from(audio.rate()) * 1e6;
                println!("64 voices (half at 44.1 kHz) for {seconds} s: {stats:?}");
                println!(
                    "block time (µs, blocks of about {period:.0} µs): p50 {} p99 {} p99.9 {} max {}; {total} blocks",
                    at(0.5),
                    at(0.99),
                    at(0.999),
                    times.iter().rposition(|&c| c > 0).unwrap_or(0)
                );
                assert_eq!(stats.voices, 64);
                assert!(stats.sounding > 0);
                silent(&audio);
            }
            other => panic!("no mode {other}"),
        }
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    probe::run();
    #[cfg(not(target_os = "macos"))]
    eprintln!("device_probe measures with macOS's counters; nothing to do here");
}
