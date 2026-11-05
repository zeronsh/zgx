//! Glass resource monitor: launches the kitchen sink, or another example with
//! `--app <example>` (or attaches with `--pid <pid>`, or neither with
//! `--app none`), and charts CPU, GPU and
//! memory over the last minute for any process on the machine. `--attach
//! <name>` launches nothing and narrows the sidebar to the processes of that
//! name and everything they started, as its Attach menu does. GPU time per
//! process comes from the GPU driver's per-client accounting (macOS: each
//! `AGXDeviceUserClient`'s `AppUsage`), the way Activity Monitor reads it.
//!
//! The interface is Luau (`examples/zgx_monitor/app.luau` and the modules it
//! requires); edit and save to reload it. It lays panels out as the terminal
//! example lays out panes: split beside or below, resized by their dividers,
//! moved by their headers onto another panel's edge (or its middle, to swap),
//! with processes dragged from the sidebar the same way.
//!
//! This file samples: every process, every half second, published to the
//! script as shared state (`procs`, `summary`), and the last minute of each
//! process a panel shows (`history:<pid>`, for the pids in `watch`).
//!
//! On macOS an app can name its own processes here, as macOS gives no way to rename a
//! running process for other tools (a browser's renderers, each drawing
//! tabs), and count things of its own. It writes
//! `<temp>/zgx-monitor/labels-<its pid>` (`std::env::temp_dir`: `$TMPDIR`),
//! replacing the whole file each time, in UTF-8 lines of:
//!
//! - `<pid>\t<label>`: the sidebar and panels call process `<pid>` (the
//!   app's or any of its children) `<label>` rather than its executable's
//!   name; the last label stays once the app stops saying;
//! - `#\t<label>\t<count>`: a total of the app's own (`Tabs`, 12), shown
//!   in the sidebar's footer as `<label> <count>`, summed over the apps in
//!   view (every one, or the attached tree) that write one so labelled.
//!
//! Other lines are skipped. The monitor touches `<temp>/zgx-monitor/heartbeat`
//! at every sample, so an app need write only while that file is fresh (a few
//! seconds old at most) and remove its own once it goes stale: labels can be
//! private (page titles), and nothing then reaches disk while no monitor runs.
//! A file whose pid has exited is removed by the monitor.
//!
//! Shipped in an app bundle (`Contents/MacOS/zgx_monitor`), it runs the
//! scripts in `Contents/Resources/monitor/`; otherwise this checkout's.
//!
//! Run: cargo run --release -p zgx-desktop --example zgx_monitor
//! (build the target first: cargo build --release -p zgx-desktop --example kitchen_sink)

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::{Child, Command},
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{reactive::Runtime, timer::sleep};
use zgx_desktop::{Application, FontData, Glass, Titlebar, WindowOptions};
use zgx_script::{ScriptHost, Value};

const TITLE: &str = "zgx monitor";
const WIDTH: f64 = 1280.;
const HEIGHT: f64 = 780.;
const INTERVAL: Duration = Duration::from_millis(500);
/// One minute of samples.
const CAPACITY: usize = 120;

#[derive(Clone, Copy, PartialEq)]
struct Sample {
    cpu: f32,
    /// Percent of one GPU's time, like `cpu` is of one core's.
    gpu: f32,
    memory_mb: f32,
}

/// Sampler state per process, kept past exit while a panel shows it.
struct Track {
    start: u64,
    name: Rc<str>,
    /// What the process's app calls it (a browser tab's title), if it says.
    label: Option<Rc<str>>,
    /// The parent when first seen.
    parent: i32,
    cpu_ns: u64,
    /// Accumulated GPU time, if the process has ever used the GPU.
    gpu_ns: Option<u64>,
    at: Instant,
    /// Smoothed CPU for a sort order that doesn't jitter every sample.
    smooth: f32,
    samples: VecDeque<Sample>,
}

/// The scripts: in the app bundle the monitor ships in, otherwise this checkout's.
fn script() -> PathBuf {
    let shipped = std::env::current_exe().ok().and_then(|exe| {
        let path = exe.parent()?.parent()?.join("Resources/monitor/app.luau");
        path.exists().then_some(path)
    });
    shipped.unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/zgx_monitor/app.luau")
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = script();
    let mut application = Application::new();
    // Geist, the interface's typeface (`font.sans` in zgx_monitor/glass.json).
    for bytes in [
        &include_bytes!("support/fonts/Geist.ttf")[..],
        include_bytes!("support/fonts/Geist-Medium.ttf"),
        include_bytes!("support/fonts/Geist-SemiBold.ttf"),
    ] {
        application = application.font(FontData::new(std::sync::Arc::<[u8]>::from(bytes))?);
    }
    application
        .window(WindowOptions {
            title: TITLE.into(),
            width: WIDTH,
            height: HEIGHT,
            decorations: cfg!(target_os = "macos"),
            min_size: Some((760., 520.)),
            titlebar: Titlebar::Inset,
            glass: Some(Glass::default()),
            ..Default::default()
        })
        .run(move |cx| {
            // Launch the target beside this binary unless attaching.
            let mut error = String::new();
            let child: Rc<RefCell<Option<Child>>> = Rc::default();
            let attach = attach_name();
            let launched = match attach_pid() {
                Some(pid) => Some(pid),
                None if target() == "none" || attach.is_some() => None,
                None => match launch_target() {
                    Ok(process) => {
                        let pid = process.id() as i32;
                        *child.borrow_mut() = Some(process);
                        Some(pid)
                    }
                    Err(message) => {
                        error = message;
                        None
                    }
                },
            };

            let host = ScriptHost::new(&cx.ui.runtime, script).app_id("zgx-monitor");
            // Panels stay concentric with the window's corners.
            let radius = host.shared("window_radius", 10.);
            let corner = cx.window_info.corner_radius.clone();
            let follow_radius = cx.ui.runtime.effect(move || {
                let r = corner.get();
                if r > 0. {
                    radius.set(r.into());
                }
            });
            host.shared(
                "launched",
                launched.map_or(Value::Nil, |pid| num(pid as f32)),
            );
            // The first panel shows the attached process, if it is running:
            // the largest, when several go by that name.
            let attach_pid = attach.as_deref().and_then(|name| {
                usage::pids()
                    .into_iter()
                    .filter(|pid| usage::name(*pid) == name)
                    .max_by_key(|pid| usage::read(*pid).map_or(0, |usage| usage.footprint))
            });
            host.shared("attach", attach.as_deref().unwrap_or(""));
            host.shared(
                "attach_pid",
                attach_pid.map_or(Value::Nil, |pid| num(pid as f32)),
            );
            host.shared("error", error.as_str());
            host.shared("interval", INTERVAL.as_millis() as f64);
            host.shared("capacity", CAPACITY as f64);
            let window = cx.window.clone();
            host.action("close_window", move |_| window.close());

            let sampler = Sampler {
                host: host.clone(),
                runtime: cx.ui.runtime.clone(),
                tracks: Rc::default(),
                procs: host.shared("procs", Value::List(Rc::new([]))),
                summary: host.shared("summary", Value::Nil),
                counts: host.shared("counts", Value::List(Rc::new([]))),
                watch: host.shared("watch", Value::List(Rc::new([]))),
            };
            // A panel opening on a process shows its last minute at once,
            // not at the next sample.
            let publish = {
                let sampler = sampler.clone();
                let watch = sampler.watch.clone();
                cx.ui.runtime.effect(move || {
                    watch.get();
                    sampler.publish_history();
                })
            };
            cx.tasks.spawn(async move {
                loop {
                    sampler.sample();
                    sleep(INTERVAL).await;
                }
            });

            let _ = host.reload();
            cx.tasks.spawn(host.watch(Duration::from_millis(250)));

            // The menus are the script's commands.
            let commands = host.commands();
            let windows = cx.windows.clone();
            let menus = cx.ui.runtime.effect(move || {
                let list = commands.get();
                if let Err(error) = windows.set_menus(zgx_run::menus(&list)) {
                    eprintln!("zgx_monitor: menus: {error}");
                }
            });
            let commands = host.clone();
            cx.on_menu_action(move |action| {
                commands.run_command(&action.0);
            });
            // The monitor owns the process it launched.
            let keep = RefCell::new(Some((menus, follow_radius, publish)));
            cx.on_closed(move || {
                drop(keep.borrow_mut().take());
                if let Some(mut process) = child.borrow_mut().take() {
                    let _ = process.kill();
                    let _ = process.wait();
                }
            });
            // `ui.window.titlebar_area`: where the title bar leaves the script room.
            host.window_chrome(
                cx.window_info.titlebar_area.clone(),
                cx.window_info.controls.clone(),
            );
            cx.render(host.view());
        })
}

fn attach_pid() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == "--pid")
        .and_then(|i| args.get(i + 1))
        .and_then(|pid| pid.parse().ok())
}

/// The process name whose tree the sidebar starts narrowed to.
fn attach_name() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == "--attach")
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The example to launch: `--app <name>`, the kitchen sink by default;
/// `--app none` launches nothing.
fn target() -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == "--app")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "kitchen_sink".into())
}

fn launch_target() -> Result<Child, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = target();
    let path = exe.with_file_name(&name);
    Command::new(&path).spawn().map_err(|e| {
        format!(
            "Could not start {} ({e}). Build it with: cargo build --release -p zgx-desktop --example {name}",
            path.display()
        )
    })
}

fn num(value: f32) -> Value {
    Value::Num(value as f64)
}

/// A script table from its fields, keys shared across every table made.
fn map<const N: usize>(fields: [(&'static str, Value); N]) -> Value {
    thread_local! {
        static KEYS: RefCell<HashMap<&'static str, Rc<str>>> = RefCell::default();
    }
    let mut fields: Vec<(Rc<str>, Value)> = KEYS.with_borrow_mut(|keys| {
        fields
            .into_iter()
            .map(|(key, value)| (keys.entry(key).or_insert_with(|| key.into()).clone(), value))
            .collect()
    });
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Map(fields.into())
}

/// Samples every readable process, so a newly opened panel already has its
/// last minute of history.
#[derive(Clone)]
struct Sampler {
    host: ScriptHost,
    runtime: Runtime,
    tracks: Rc<RefCell<HashMap<i32, Track>>>,
    /// `{ pid, ppid, name, cpu, gpu, mem, smooth }` for every live process.
    procs: zgx_script::Shared,
    /// `{ count, cpu, gpu }`: the whole machine.
    summary: zgx_script::Shared,
    /// `{ owner, label, count }`: what apps count of their own.
    counts: zgx_script::Shared,
    /// The pids the script's panels show (it writes this).
    watch: zgx_script::Shared,
}
impl Sampler {
    fn watched(&self) -> HashSet<i32> {
        self.watch.with_untracked(|watch| {
            watch
                .list()
                .iter()
                .filter_map(Value::as_f64)
                .map(|pid| pid as i32)
                .collect()
        })
    }
    fn sample(&self) {
        let watched = self.watched();
        let labels = labels::read();
        let procs = sample(&mut self.tracks.borrow_mut(), &watched, &labels.names);
        let counts: Vec<Value> = labels
            .counts
            .into_iter()
            .map(|(owner, label, count)| {
                map([
                    ("owner", num(owner as f32)),
                    ("label", Value::Str(label)),
                    ("count", num(count)),
                ])
            })
            .collect();
        let cpu: f32 = procs.iter().map(|p| p.2).sum();
        let summary = map([
            ("count", num(procs.len() as f32)),
            ("cpu", num(cpu)),
            ("gpu", usage::device_gpu().map_or(Value::Nil, num)),
        ]);
        let rows: Vec<Value> = procs
            .into_iter()
            .map(|(pid, name, cpu, gpu, memory_mb, smooth, parent, label)| {
                map([
                    ("pid", num(pid as f32)),
                    ("ppid", num(parent as f32)),
                    ("name", Value::Str(name)),
                    ("label", label.map_or(Value::Nil, Value::Str)),
                    ("cpu", num(cpu)),
                    ("gpu", num(gpu)),
                    ("mem", num(memory_mb)),
                    ("smooth", num(smooth)),
                ])
            })
            .collect();
        self.runtime.batch(|| {
            self.procs.set(Value::List(rows.into()));
            self.summary.set(summary);
            self.counts.set(Value::List(counts.into()));
            self.publish_history();
        });
    }
    /// Each watched process's last minute, as `history:<pid>`:
    /// `{ name, cpu = { … }, gpu = { … }, mem = { … } }`, oldest first.
    fn publish_history(&self) {
        let tracks = self.tracks.borrow();
        for pid in self.watched() {
            let Some(track) = tracks.get(&pid) else {
                continue;
            };
            let series = |read: fn(&Sample) -> f32| {
                Value::List(track.samples.iter().map(|s| num(read(s))).collect())
            };
            let history = map([
                ("name", Value::Str(track.name.clone())),
                ("label", track.label.clone().map_or(Value::Nil, Value::Str)),
                ("cpu", series(|s| s.cpu)),
                ("gpu", series(|s| s.gpu)),
                ("mem", series(|s| s.memory_mb)),
            ]);
            self.host
                .shared(&format!("history:{pid}"), Value::Nil)
                .set(history);
        }
    }
}

/// A process's reading: pid, name, CPU, GPU, memory (MB), the smoothed CPU,
/// the parent's pid and the label its app gave it.
type Reading = (i32, Rc<str>, f32, f32, f32, f32, i32, Option<Rc<str>>);

/// One reading per live process.
fn sample(
    tracks: &mut HashMap<i32, Track>,
    open: &HashSet<i32>,
    labels: &HashMap<i32, Rc<str>>,
) -> Vec<Reading> {
    let now = Instant::now();
    let mut seen = HashSet::new();
    let mut procs = Vec::new();
    let gpu_times = usage::gpu_times();
    for pid in usage::pids() {
        let Some(usage) = usage::read(pid) else {
            continue;
        };
        seen.insert(pid);
        let label = labels.get(&pid).cloned();
        let memory_mb = usage.footprint as f32 / (1024. * 1024.);
        let track = match tracks.get_mut(&pid) {
            // A reused pid is a different process.
            Some(track) if track.start == usage.start => track,
            _ => {
                let name: Rc<str> = usage::name(pid).into();
                let parent = usage::parent(pid);
                tracks.insert(
                    pid,
                    Track {
                        start: usage.start,
                        name: name.clone(),
                        label: label.clone(),
                        parent,
                        cpu_ns: usage.cpu_ns,
                        gpu_ns: gpu_times.get(&pid).copied(),
                        at: now,
                        smooth: 0.,
                        samples: VecDeque::new(),
                    },
                );
                procs.push((pid, name, 0., 0., memory_mb, 0., parent, label));
                continue;
            }
        };
        // Kept once the app stops saying, as when it quits with its helpers still going.
        if label.is_some() {
            track.label = label.clone();
        }
        let wall = now.duration_since(track.at).as_nanos().max(1) as f64;
        let cpu = (usage.cpu_ns.saturating_sub(track.cpu_ns) as f64 / wall * 100.) as f32;
        track.cpu_ns = usage.cpu_ns;
        let gpu_ns = gpu_times.get(&pid).copied();
        // A process that closes its last GPU client drops back to zero.
        let gpu = match (gpu_ns, track.gpu_ns) {
            (Some(now_ns), Some(then)) => (now_ns.saturating_sub(then) as f64 / wall * 100.) as f32,
            _ => 0.,
        };
        track.gpu_ns = gpu_ns;
        track.at = now;
        track.smooth = track.smooth * 0.8 + cpu * 0.2;
        if track.samples.len() == CAPACITY {
            track.samples.pop_front();
        }
        track.samples.push_back(Sample {
            cpu,
            gpu,
            memory_mb,
        });
        procs.push((
            pid,
            track.name.clone(),
            cpu,
            gpu,
            memory_mb,
            track.smooth,
            track.parent,
            track.label.clone(),
        ));
    }
    // Exited processes keep their history while a panel shows them.
    tracks.retain(|pid, _| seen.contains(pid) || open.contains(pid));
    procs
}

/// What apps call their own processes: a process can't be renamed for other
/// tools on macOS, so an app that wants its helpers told apart writes
/// `<temp>/zgx-monitor/labels-<its pid>`, one `pid<TAB>label` per line, and
/// `#<TAB>label<TAB>count` for a total of its own (its open tabs, say). The
/// heartbeat says someone is reading; an app stops writing when it goes stale.
#[cfg(target_os = "macos")]
mod labels {
    use std::collections::HashMap;
    use std::os::unix::fs::DirBuilderExt;
    use std::rc::Rc;
    use std::time::SystemTime;

    #[derive(Default)]
    pub struct Labels {
        pub names: HashMap<i32, Rc<str>>,
        /// An app's pid, what it counts and how many.
        pub counts: Vec<(i32, Rc<str>, f32)>,
    }

    pub fn read() -> Labels {
        let mut labels = Labels::default();
        let directory = std::env::temp_dir().join("zgx-monitor");
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory);
        let heartbeat = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(directory.join("heartbeat"));
        if let Ok(file) = heartbeat {
            let _ = file.set_modified(SystemTime::now());
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return labels;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(owner) = name
                .to_str()
                .and_then(|name| name.strip_prefix("labels-"))
                .and_then(|pid| pid.parse::<i32>().ok())
            else {
                continue;
            };
            // Left by an app that has quit.
            if super::usage::read(owner).is_none() {
                let _ = std::fs::remove_file(entry.path());
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for line in text.lines() {
                if let Some(count) = line.strip_prefix("#\t") {
                    if let Some((label, count)) = count.split_once('\t')
                        && let Ok(count) = count.parse()
                    {
                        labels.counts.push((owner, label.into(), count));
                    }
                } else if let Some((pid, label)) = line.split_once('\t')
                    && let Ok(pid) = pid.parse()
                    && !label.is_empty()
                {
                    labels.names.insert(pid, label.into());
                }
            }
        }
        labels
    }
}
#[cfg(not(target_os = "macos"))]
mod labels {
    #[derive(Default)]
    pub struct Labels {
        pub names: std::collections::HashMap<i32, std::rc::Rc<str>>,
        pub counts: Vec<(i32, std::rc::Rc<str>, f32)>,
    }
    pub fn read() -> Labels {
        Labels::default()
    }
}

/// Per-process CPU time and physical footprint (what Activity Monitor shows).
#[cfg(target_os = "macos")]
mod usage {
    use std::sync::OnceLock;
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV2 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
        child_user_time: u64,
        child_system_time: u64,
        child_pkg_idle_wkups: u64,
        child_interrupt_wkups: u64,
        child_pageins: u64,
        child_elapsed_abstime: u64,
        diskio_bytesread: u64,
        diskio_byteswritten: u64,
    }
    /// `proc_bsdshortinfo`, up to the parent's pid being what's read of it.
    #[repr(C)]
    #[derive(Default)]
    struct ShortInfo {
        pid: u32,
        ppid: u32,
        pgid: u32,
        status: u32,
        comm: [u8; 16],
        flags: u32,
        ids: [u32; 6],
        reserved: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    // SAFETY: libproc's and Mach's declarations (libproc.h, mach_time.h);
    // `RusageInfoV2` is `rusage_info_v2` and `ShortInfo` is `proc_bsdshortinfo`
    // field for field.
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV2) -> i32;
        fn proc_listallpids(buffer: *mut i32, size: i32) -> i32;
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut ShortInfo, size: i32) -> i32;
        fn proc_pidpath(pid: i32, buffer: *mut u8, size: u32) -> i32;
        fn proc_name(pid: i32, buffer: *mut u8, size: u32) -> i32;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    const RUSAGE_INFO_V2: i32 = 2;
    const PROC_PIDT_SHORTBSDINFO: i32 = 13;

    pub struct Usage {
        pub cpu_ns: u64,
        pub footprint: u64,
        /// Distinguishes a reused pid.
        pub start: u64,
    }
    pub fn pids() -> Vec<i32> {
        // SAFETY: a null buffer asks for the current count.
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) }.max(0) as usize;
        // Room for processes started between the two calls.
        let mut pids = vec![0_i32; count + 64];
        let bytes = (pids.len() * size_of::<i32>()) as i32;
        // SAFETY: the buffer holds `bytes` writable bytes.
        let count = unsafe { proc_listallpids(pids.as_mut_ptr(), bytes) }.max(0) as usize;
        pids.truncate(count.min(pids.len()));
        pids.retain(|pid| *pid > 0);
        pids
    }
    /// The parent's pid, or 0 if it can't be read.
    pub fn parent(pid: i32) -> i32 {
        let mut info = ShortInfo::default();
        let size = size_of::<ShortInfo>() as i32;
        // SAFETY: the buffer is a correctly sized, writable proc_bsdshortinfo.
        let read = unsafe { proc_pidinfo(pid, PROC_PIDT_SHORTBSDINFO, 0, &mut info, size) };
        if read == size { info.ppid as i32 } else { 0 }
    }
    /// The executable's file name (proc_name truncates at 32 bytes).
    pub fn name(pid: i32) -> String {
        let mut buffer = [0_u8; 4096];
        // SAFETY: the buffer is writable for its full length (4096 bytes,
        // PROC_PIDPATHINFO_MAXSIZE); the length returned is within it.
        let len = unsafe { proc_pidpath(pid, buffer.as_mut_ptr(), buffer.len() as u32) };
        if len > 0 {
            let path = String::from_utf8_lossy(&buffer[..len as usize]);
            if let Some(name) = path.rsplit('/').next().filter(|n| !n.is_empty()) {
                return name.to_owned();
            }
        }
        // SAFETY: as above.
        let len = unsafe { proc_name(pid, buffer.as_mut_ptr(), buffer.len() as u32) };
        if len > 0 {
            return String::from_utf8_lossy(&buffer[..len as usize]).into_owned();
        }
        format!("pid {pid}")
    }
    pub fn read(pid: i32) -> Option<Usage> {
        static TIMEBASE: OnceLock<(u32, u32)> = OnceLock::new();
        let mut info = RusageInfoV2::default();
        // SAFETY: the buffer is a correctly sized, writable rusage_info_v2.
        if unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V2, &mut info) } != 0 {
            return None;
        }
        // CPU times are in Mach absolute-time units (not ns on Apple silicon).
        let &(numer, denom) = TIMEBASE.get_or_init(|| {
            let mut timebase = Timebase::default();
            // SAFETY: writes two integers into a local.
            unsafe { mach_timebase_info(&mut timebase) };
            (timebase.numer, timebase.denom.max(1))
        });
        let ticks = info.user_time + info.system_time;
        let cpu_ns = (ticks as u128 * numer as u128 / denom as u128) as u64;
        Some(Usage {
            cpu_ns,
            footprint: info.phys_footprint,
            start: info.proc_start_abstime,
        })
    }

    // GPU time per process -----------------------------------------------------
    //
    // The GPU driver keeps per-client accounting in the IORegistry: every
    // `AGXDeviceUserClient` names its creator ("pid 395, WindowServer") and
    // lists `AppUsage` entries with `accumulatedGPUTime` in nanoseconds.
    // `IOAccelerator`'s `PerformanceStatistics` hold the device-wide
    // "Device Utilization %". Both are readable without privileges.
    use std::{
        collections::HashMap,
        ffi::{CStr, c_char, c_void},
    };
    type CfType = *const c_void;
    // SAFETY: IOKit's and CoreFoundation's declarations (IOKitLib.h,
    // CFString.h, CFArray.h, CFDictionary.h, CFNumber.h): io_object_t and
    // mach ports are u32, kern_return_t i32, CFIndex isize, CFTypeID usize,
    // CFStringEncoding u32, Boolean a byte that is 0 or 1.
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingServices(main: u32, matching: *mut c_void, it: *mut u32) -> i32;
        fn IOIteratorNext(it: u32) -> u32;
        fn IOObjectRelease(object: u32) -> i32;
        fn IORegistryEntryGetChildIterator(entry: u32, plane: *const c_char, it: *mut u32) -> i32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: CfType,
            allocator: CfType,
            options: u32,
        ) -> CfType;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: CfType,
            text: *const c_char,
            encoding: u32,
        ) -> CfType;
        fn CFStringGetCString(
            text: CfType,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> bool;
        fn CFGetTypeID(object: CfType) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFArrayGetCount(array: CfType) -> isize;
        fn CFArrayGetValueAtIndex(array: CfType, index: isize) -> CfType;
        fn CFDictionaryGetValue(dictionary: CfType, key: CfType) -> CfType;
        fn CFNumberGetValue(number: CfType, kind: isize, value: *mut c_void) -> bool;
        fn CFRelease(object: CfType);
    }
    const UTF8: u32 = 0x0800_0100;
    const SINT64: isize = 4;

    /// An owned CFString key.
    struct Key(CfType);
    impl Key {
        fn new(text: &CStr) -> Self {
            // SAFETY: a NUL-terminated UTF-8 string; released in Drop.
            Self(unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) })
        }
    }
    impl Drop for Key {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: created above and not shared.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// Calls `visit` with each registry entry of `class`, then releases it.
    fn each_service(class: &CStr, mut visit: impl FnMut(u32)) {
        let mut iterator = 0_u32;
        // SAFETY: `IOServiceGetMatchingServices` consumes the matching
        // dictionary; the iterator and entries are released here.
        unsafe {
            let matching = IOServiceMatching(class.as_ptr());
            if matching.is_null() || IOServiceGetMatchingServices(0, matching, &mut iterator) != 0 {
                return;
            }
            loop {
                let entry = IOIteratorNext(iterator);
                if entry == 0 {
                    break;
                }
                visit(entry);
                IOObjectRelease(entry);
            }
            IOObjectRelease(iterator);
        }
    }

    /// Calls `visit` with each child of `entry` in the service plane. User
    /// clients are not registered services, so matching can't find them;
    /// they hang off their accelerator like `ioreg` shows.
    fn each_child(entry: u32, mut visit: impl FnMut(u32)) {
        let mut iterator = 0_u32;
        // SAFETY: a valid entry; the iterator and children are released.
        unsafe {
            if IORegistryEntryGetChildIterator(entry, c"IOService".as_ptr(), &mut iterator) != 0 {
                return;
            }
            loop {
                let child = IOIteratorNext(iterator);
                if child == 0 {
                    break;
                }
                visit(child);
                IOObjectRelease(child);
            }
            IOObjectRelease(iterator);
        }
    }

    /// A copied property of `entry`, released when dropped.
    struct Property(CfType);
    impl Property {
        fn of(entry: u32, key: &Key) -> Option<Self> {
            // SAFETY: a valid entry and key; the result is owned (Create rule).
            let value =
                unsafe { IORegistryEntryCreateCFProperty(entry, key.0, std::ptr::null(), 0) };
            // Lazily: an eager `Self(value)` would be dropped, releasing
            // null, when the entry lacks the property.
            (!value.is_null()).then(|| Self(value))
        }
    }
    impl Drop for Property {
        fn drop(&mut self) {
            // SAFETY: owned since `of`.
            unsafe { CFRelease(self.0) };
        }
    }

    /// # Safety
    ///
    /// `object` is null or a live CF object.
    unsafe fn is(object: CfType, type_id: usize) -> bool {
        // SAFETY: non-null, so a live CF object (the caller's promise).
        !object.is_null() && unsafe { CFGetTypeID(object) } == type_id
    }

    /// # Safety
    ///
    /// As `is`'s.
    unsafe fn number(object: CfType) -> Option<i64> {
        // SAFETY: the caller's object, read as a CFNumber only once it is
        // one, into an i64 (kCFNumberSInt64Type).
        unsafe {
            if !is(object, CFNumberGetTypeID()) {
                return None;
            }
            let mut value = 0_i64;
            CFNumberGetValue(object, SINT64, (&raw mut value).cast()).then_some(value)
        }
    }

    /// # Safety
    ///
    /// As `is`'s.
    unsafe fn string(object: CfType) -> Option<String> {
        // SAFETY: the caller's object, read as a CFString only once it is
        // one; CoreFoundation writes at most the buffer's length, NUL
        // included, and only a `true` (it fit) is read back.
        unsafe {
            if !is(object, CFStringGetTypeID()) {
                return None;
            }
            let mut buffer = [0 as c_char; 256];
            CFStringGetCString(object, buffer.as_mut_ptr(), buffer.len() as isize, UTF8).then(
                || {
                    CStr::from_ptr(buffer.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                },
            )
        }
    }

    /// "pid 395, WindowServer" → 395.
    pub fn creator_pid(creator: &str) -> Option<i32> {
        creator
            .strip_prefix("pid ")?
            .split(',')
            .next()?
            .trim()
            .parse()
            .ok()
    }

    /// Accumulated GPU nanoseconds per process with an open GPU client.
    pub fn gpu_times() -> HashMap<i32, u64> {
        let (creator_key, usage_key, time_key) = (
            Key::new(c"IOUserClientCreator"),
            Key::new(c"AppUsage"),
            Key::new(c"accumulatedGPUTime"),
        );
        let mut times = HashMap::new();
        let mut read = |entry: u32| {
            // SAFETY: a property is a live CF object while held.
            let Some(pid) = Property::of(entry, &creator_key)
                .and_then(|creator| unsafe { string(creator.0) })
                .and_then(|creator| creator_pid(&creator))
            else {
                return;
            };
            let Some(usage) = Property::of(entry, &usage_key) else {
                return;
            };
            // SAFETY: `usage` (live while held) is read as an array only once
            // it is one, at indices within its count; its items and their
            // values are Get-rule references it keeps alive meanwhile.
            let total: u64 = unsafe {
                if !is(usage.0, CFArrayGetTypeID()) {
                    return;
                }
                (0..CFArrayGetCount(usage.0))
                    .map(|i| CFArrayGetValueAtIndex(usage.0, i))
                    .filter(|item| is(*item, CFDictionaryGetTypeID()))
                    .filter_map(|item| number(CFDictionaryGetValue(item, time_key.0)))
                    .map(|ns| ns.max(0) as u64)
                    .sum()
            };
            *times.entry(pid).or_insert(0) += total;
        };
        each_service(c"IOAccelerator", |accelerator| {
            each_child(accelerator, &mut read)
        });
        times
    }

    /// The GPU's own "Device Utilization %", if the driver reports it.
    pub fn device_gpu() -> Option<f32> {
        let (stats_key, utilization_key) = (
            Key::new(c"PerformanceStatistics"),
            Key::new(c"Device Utilization %"),
        );
        let mut found = None;
        each_service(c"IOAccelerator", |entry| {
            if found.is_some() {
                return;
            }
            if let Some(stats) = Property::of(entry, &stats_key)
                // SAFETY: `stats` is a live CF object while held.
                && unsafe { is(stats.0, CFDictionaryGetTypeID()) }
            {
                // SAFETY: a dictionary, checked above; the value it holds is a
                // Get-rule reference, live while `stats` is.
                found = unsafe { number(CFDictionaryGetValue(stats.0, utilization_key.0)) }
                    .map(|percent| percent as f32);
            }
        });
        found
    }
}

#[cfg(not(target_os = "macos"))]
mod usage {
    /// No per-process GPU accounting here yet.
    pub fn gpu_times() -> std::collections::HashMap<i32, u64> {
        std::collections::HashMap::new()
    }
    pub fn device_gpu() -> Option<f32> {
        None
    }
    pub struct Usage {
        pub cpu_ns: u64,
        pub footprint: u64,
        pub start: u64,
    }
    pub fn pids() -> Vec<i32> {
        std::fs::read_dir("/proc")
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn name(pid: i32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|name| name.trim_end().to_owned())
            .unwrap_or_else(|_| format!("pid {pid}"))
    }
    /// The parent's pid, or 0 if it can't be read.
    pub fn parent(pid: i32) -> i32 {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit(')')
                    .next()?
                    .split_whitespace()
                    .nth(1)?
                    .parse()
                    .ok()
            })
            .unwrap_or(0)
    }
    /// Linux: /proc/<pid>/stat CPU ticks (assumed 100 Hz) and resident memory.
    pub fn read(pid: i32) -> Option<Usage> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields: Vec<&str> = stat.rsplit(')').next()?.split_whitespace().collect();
        let ticks: u64 =
            fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
        let pages: u64 = fields.get(21)?.parse().ok()?;
        Some(Usage {
            cpu_ns: ticks * 10_000_000,
            footprint: pages * 4096,
            start: fields.get(19)?.parse().ok()?,
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::usage;

    #[test]
    fn creator_names_its_pid() {
        assert_eq!(usage::creator_pid("pid 395, WindowServer"), Some(395));
        assert_eq!(usage::creator_pid("pid 7,x"), Some(7));
        assert_eq!(usage::creator_pid("WindowServer"), None);
    }

    #[test]
    fn parent_reads_this_process_parent() {
        let pid = std::process::id() as i32;
        assert_eq!(
            usage::parent(pid),
            std::os::unix::process::parent_id() as i32
        );
    }

    #[test]
    fn window_server_has_gpu_time() {
        // Every logged-in Mac composites through WindowServer on the GPU.
        let times = usage::gpu_times();
        assert!(times.values().any(|ns| *ns > 0), "no GPU accounting read");
    }
}
