//! What the device costs and how fast it starts, on macOS and Windows, at zero gain:
//! master's gain is 0 from the first frame, so streams run and counters
//! count but nothing is heard. Every mode checks the output stayed silent.
//!
//! `cargo run --release -p zgx-audio --features device --example device_probe -- <mode>`
//!
//! - `none <s>`: the process's wakeups over `s` seconds, no audio at all
//! - `idle <s>`: the same with an engine made and a sound loaded, nothing played
//! - `after <s> [pause]`: one click played, its stream stopped after the hold, then `s` seconds
//!   (paused: then again once the grace, `Config::close_after`, has closed it)
//! - `latency <n> <pause|close>`: `n` cold starts and `3n` plays with the stream running, play to heard
//! - `threads <n>`: after one click, `n` windows of 30 s: wakeups and each thread's CPU time
//!   (`queried`: only the default output's format asked; `streamed`: a stream played to its end)
//! - `load <s>`: 64 voices looping for `s` seconds: block times, underruns
//!
//! On Windows (WASAPI, shared mode) the same modes print the same lines:
//! "wakeups" there are the context switches of the process's threads (the
//! times they were woken to run), thread times are kernel plus user (counted
//! in scheduler ticks, 15.6 ms), and the clock is the performance counter
//! WASAPI's timestamps count. `latency` fails there after its 5 s wait for
//! the first click: a cold start's first block reaches the mixer stamped 0
//! (cpal stamps WASAPI's pre-fill from a clock not yet running), so a play
//! that starts the stream is never given a time.
#[cfg(any(target_os = "macos", windows))]
mod probe {
    use std::time::{Duration, Instant};
    use zgx_audio::{Audio, Config, Cpal, Play, Sound, Stopping};

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct RusageInfoV4 {
        uuid: [u8; 16],
        fields: [u64; 40],
    }

    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV4) -> i32;
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, size: i32) -> i32;
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }

    /// Package-idle and interrupt wakeups of this process so far.
    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct ThreadExtendedInfo {
        user_time: u64,
        system_time: u64,
        cpu_usage: i32,
        policy: i32,
        run_state: i32,
        flags: i32,
        sleep_time: i32,
        curpri: i32,
        priority: i32,
        maxpriority: i32,
        name: [u8; 64],
    }

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct ThreadIdentifierInfo {
        thread_id: u64,
        handle: u64,
        dispatch_qaddr: u64,
    }

    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_threads(task: u32, threads: *mut *mut u32, count: *mut u32) -> i32;
        fn thread_info(thread: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }

    /// Each of this process's threads: its id, name, and CPU time so far
    /// (user plus system, in nanoseconds).
    #[cfg(target_os = "macos")]
    fn thread_times() -> Vec<(u64, String, u64)> {
        let mut list: *mut u32 = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: task_threads fills a list of this task's thread ports;
        // THREAD_EXTENDED_INFO (5) and THREAD_IDENTIFIER_INFO (4) fill the
        // structs above, whose sizes in integers are passed.
        unsafe {
            assert_eq!(task_threads(mach_task_self_, &mut list, &mut count), 0);
            (0..count as usize)
                .map(|i| {
                    let thread = *list.add(i);
                    let mut info: ThreadExtendedInfo = std::mem::zeroed();
                    let mut size = (size_of::<ThreadExtendedInfo>() / 4) as u32;
                    thread_info(thread, 5, &mut info as *mut _ as *mut i32, &mut size);
                    let mut id: ThreadIdentifierInfo = std::mem::zeroed();
                    let mut size = (size_of::<ThreadIdentifierInfo>() / 4) as u32;
                    thread_info(thread, 4, &mut id as *mut _ as *mut i32, &mut size);
                    let end = info.name.iter().position(|&b| b == 0).unwrap_or(64);
                    let name = String::from_utf8_lossy(&info.name[..end]).into_owned();
                    (id.thread_id, name, info.user_time + info.system_time)
                })
                .collect()
        }
    }

    /// This process's threads.
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    fn clock() -> u64 {
        // SAFETY: a clock id the system defines.
        unsafe { clock_gettime_nsec_np(8) }
    }

    #[cfg(windows)]
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(class: u32, info: *mut u8, length: u32, out: *mut u32) -> i32;
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(count: *mut i64) -> i32;
        fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
        fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
        fn GetThreadDescription(thread: isize, description: *mut *mut u16) -> i32;
        fn LocalFree(memory: *mut u16) -> *mut u16;
        fn CloseHandle(handle: isize) -> i32;
    }

    /// Each of this process's threads: its id, its context switches and its
    /// CPU time (kernel plus user, in nanoseconds) so far, as the kernel
    /// lists them (SystemProcessInformation).
    #[cfg(windows)]
    fn snapshot() -> Vec<(u64, u64, u64)> {
        // u64s, for the structures' alignment.
        let mut buffer = vec![0u64; 1 << 17];
        loop {
            let mut needed = 0u32;
            let bytes = (buffer.len() * 8) as u32;
            // SAFETY: class 5 fills the buffer with SYSTEM_PROCESS_INFORMATION
            // entries, writing no more than its length, and says how much it
            // needed where it was too short (STATUS_INFO_LENGTH_MISMATCH).
            let status = unsafe {
                NtQuerySystemInformation(5, buffer.as_mut_ptr().cast(), bytes, &mut needed)
            };
            if status == 0xC000_0004_u32 as i32 {
                buffer.resize(buffer.len().max(needed as usize / 8) * 2, 0);
                continue;
            }
            assert_eq!(status, 0, "NtQuerySystemInformation: {status:#x}");
            break;
        }
        // SAFETY: the buffer's own bytes, as many as it holds.
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast(), buffer.len() * 8) };
        let read = |at: usize, size: usize| {
            let mut word = [0u8; 8];
            word[..size].copy_from_slice(&bytes[at..at + size]);
            u64::from_le_bytes(word)
        };
        // x64 layouts: a process entry is 256 bytes (NextEntryOffset at 0,
        // NumberOfThreads at 4, UniqueProcessId at 80), its threads' 80 each
        // after it (KernelTime at 0, UserTime at 8, UniqueThread at 48,
        // ContextSwitches at 64; times in 100 ns).
        let me = u64::from(std::process::id());
        let mut entry = 0;
        loop {
            if read(entry + 80, 8) == me {
                return (0..read(entry + 4, 4) as usize)
                    .map(|i| {
                        let thread = entry + 256 + i * 80;
                        let time = (read(thread, 8) + read(thread + 8, 8)) * 100;
                        (read(thread + 48, 8), read(thread + 64, 4), time)
                    })
                    .collect();
            }
            let next = read(entry, 4) as usize;
            assert!(next != 0, "this process isn't listed");
            entry += next;
        }
    }

    /// Context switches of this process's threads so far, those that have
    /// exited included (each thread's last count is kept).
    #[cfg(windows)]
    fn wakeups() -> u64 {
        static SEEN: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());
        let mut seen = SEEN.lock().unwrap();
        for (id, switches, _) in snapshot() {
            match seen.iter_mut().find(|(seen, _)| *seen == id) {
                Some(entry) => entry.1 = entry.1.max(switches),
                None => seen.push((id, switches)),
            }
        }
        seen.iter().map(|&(_, switches)| switches).sum()
    }

    /// Each of this process's threads: its id, name, and CPU time so far
    /// (kernel plus user, in nanoseconds).
    #[cfg(windows)]
    fn thread_times() -> Vec<(u64, String, u64)> {
        snapshot()
            .into_iter()
            .map(|(id, _, time)| {
                let mut name = String::new();
                // SAFETY: a handle opened here to read the thread's name
                // (THREAD_QUERY_LIMITED_INFORMATION), closed here; the name
                // is a NUL-terminated string the system allocates and
                // LocalFree releases.
                unsafe {
                    let thread = OpenThread(0x0800, 0, id as u32);
                    if thread != 0 {
                        let mut text: *mut u16 = std::ptr::null_mut();
                        if GetThreadDescription(thread, &mut text) >= 0 && !text.is_null() {
                            let length = (0..).take_while(|&i| *text.add(i) != 0).count();
                            name =
                                String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
                            LocalFree(text);
                        }
                        CloseHandle(thread);
                    }
                }
                (id, name, time)
            })
            .collect()
    }

    /// This process's threads.
    #[cfg(windows)]
    fn threads() -> i32 {
        snapshot().len() as i32
    }

    /// The clock WASAPI's timestamps count (the performance counter), in
    /// nanoseconds on the 100 ns grid cpal puts them on.
    #[cfg(windows)]
    fn clock() -> u64 {
        let (mut count, mut frequency) = (0i64, 0i64);
        // SAFETY: both write the integer given.
        unsafe {
            QueryPerformanceCounter(&mut count);
            QueryPerformanceFrequency(&mut frequency);
        }
        (count as u128 * 10_000_000 / frequency as u128 * 100) as u64
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
                if stopping == Stopping::Pause {
                    // Paused, it's closed after its grace: then measured again.
                    let grace = Config::default().close_after;
                    std::thread::sleep(
                        (grace + Duration::from_secs(1))
                            .saturating_sub(Duration::from_secs(number(1, 60))),
                    );
                    let (wakes, threads) = measure(number(1, 60));
                    println!(
                        "after its grace ({grace:?}), closed: {wakes} wakeups, {threads} threads"
                    );
                }
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
            "threads" | "queried" | "streamed" => {
                // After one click and its stream closed (or, `queried`, only
                // the default output's format asked, no stream ever): every
                // 30 s, the process's wakeups and each thread's CPU time.
                let audio = engine(stopping);
                if mode == "queried" {
                    use zgx_audio::Device;
                    println!("default output: {:?}", Cpal::new().format());
                } else if mode == "streamed" {
                    // A second of noise streamed to its end: the decode
                    // thread fills it, then sleeps with nothing open.
                    let noise: Vec<f32> = (0..48_000)
                        .map(|i| ((i * 7919 % 1000) as f32 / 1000. - 0.5) * 0.1)
                        .collect();
                    let bytes =
                        zgx_audio::wav::write(&noise, 1, 48_000, zgx_audio::wav::Encoding::Float32)
                            .expect("a WAV of noise");
                    let voice = audio
                        .stream(zgx_audio::Source::bytes(bytes), Play::new())
                        .unwrap();
                    wait("the stream to start", Duration::from_secs(5), || {
                        audio.stats().running
                    });
                    wait("the stream to end", Duration::from_secs(5), || {
                        !voice.playing()
                    });
                    wait(
                        "the decode thread to let it go",
                        Duration::from_secs(5),
                        || audio.stats().streams == 0,
                    );
                    wait("the device to stop", Duration::from_secs(5), || {
                        audio.stats().stops == 1
                    });
                    assert_eq!(audio.stats().dry, 0, "{:?}", audio.stats());
                } else {
                    let sound = click();
                    audio.play(&sound, Play::new()).unwrap();
                    wait("the stream to start", Duration::from_secs(5), || {
                        audio.stats().running
                    });
                    wait("the stream to stop", Duration::from_secs(5), || {
                        audio.stats().stops == 1
                    });
                }
                let windows = number(1, 10);
                let mut before = thread_times();
                for window in 0..windows {
                    let wakes = wakeups();
                    std::thread::sleep(Duration::from_secs(30));
                    let wakes = wakeups() - wakes;
                    let now = thread_times();
                    let busy: Vec<String> = now
                        .iter()
                        .map(|(id, name, time)| {
                            let was = before
                                .iter()
                                .find(|(b, ..)| b == id)
                                .map_or(0, |(.., t)| *t);
                            let name = if name.is_empty() { "?" } else { name };
                            format!("{name}#{} +{}µs", id % 10_000, (time - was) / 1000)
                        })
                        .collect();
                    println!(
                        "{:>4} s: {wakes} wakeups; {}",
                        (window + 1) * 30,
                        busy.join(", ")
                    );
                    before = now;
                }
                if mode != "queried" {
                    silent(&audio);
                }
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
    #[cfg(any(target_os = "macos", windows))]
    probe::run();
    #[cfg(not(any(target_os = "macos", windows)))]
    eprintln!("device_probe measures with macOS's and Windows' counters; nothing to do here");
}
