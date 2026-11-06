//! `ZGX_IOS_METRICS=seconds`: every so often a line of what the app costs on
//! the device it runs on, to stdout and to `Documents/zgx-metrics.jsonl` (which
//! `scripts/ios/device.py measure` fetches). Display-link gaps say how late
//! the main thread is, the rest what the process and its GPU work take. The
//! compositor's share of the GPU, glass's above all, is not in here: for that,
//! Instruments (docs/ios-device.md).
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
    sel,
};
use std::{
    cell::RefCell,
    io::Write,
    time::{Duration, Instant},
};

unsafe extern "C" {
    static mach_task_self_: u32;
    static NSRunLoopCommonModes: *const AnyObject;
    fn task_info(task: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
    fn getrusage(who: i32, usage: *mut [i64; 18]) -> i32;
}

/// The process's memory as the system charges it, in MB.
fn footprint_mb() -> f64 {
    const TASK_VM_INFO: i32 = 22;
    // `task_vm_info`: `phys_footprint` is the u64 at byte 144.
    let mut info = [0u32; 128];
    let mut count = info.len() as u32;
    // SAFETY: a buffer as long as `count` says, for the kernel to fill.
    let ok = unsafe { task_info(mach_task_self_, TASK_VM_INFO, info.as_mut_ptr(), &mut count) };
    if ok != 0 {
        return -1.;
    }
    let bytes = u64::from(info[36]) | (u64::from(info[37]) << 32);
    bytes as f64 / 1_048_576.
}

/// User and system CPU seconds so far.
fn cpu_seconds() -> f64 {
    let mut usage = [0i64; 18];
    // SAFETY: `rusage` begins with two timevals; this buffer holds it all.
    unsafe { getrusage(0, &mut usage) };
    let (user, system) = (usage[0], usage[2]);
    user as f64 + usage[1] as i32 as f64 / 1e6 + system as f64 + usage[3] as i32 as f64 / 1e6
}

struct Window {
    since: Instant,
    last_tick: Option<Instant>,
    gaps_ms: Vec<f32>,
    cpu: f64,
    gpu_ns: u64,
    gpu_buffers: u64,
    every: Duration,
    scene: String,
    log: Option<std::fs::File>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Window>]
    struct Sampler;
    unsafe impl NSObjectProtocol for Sampler {}
    impl Sampler {
        #[unsafe(method(tick:))]
        fn tick(&self, _link: &AnyObject) {
            let mut window = self.ivars().borrow_mut();
            let now = Instant::now();
            if let Some(last) = window.last_tick.replace(now) {
                window.gaps_ms.push(now.duration_since(last).as_secs_f32() * 1000.);
            }
            if now.duration_since(window.since) >= window.every {
                report(&mut window, now);
            }
        }
    }
);

fn report(window: &mut Window, now: Instant) {
    let elapsed = now.duration_since(window.since).as_secs_f64();
    let cpu = cpu_seconds();
    let (gpu_ns, gpu_buffers) = zgx_gpu::metal_gpu_time();
    window.gaps_ms.sort_by(f32::total_cmp);
    let gaps = &window.gaps_ms;
    let at = |q: f32| {
        gaps.get(((gaps.len() as f32 - 1.) * q) as usize)
            .copied()
            .unwrap_or(0.)
    };
    // SAFETY: NSProcessInfo's and UIDevice's getters.
    let (thermal, battery) = unsafe {
        let info: Retained<AnyObject> = msg_send![class!(NSProcessInfo), processInfo];
        let thermal: isize = msg_send![&*info, thermalState];
        let device: Retained<AnyObject> = msg_send![class!(UIDevice), currentDevice];
        let _: () = msg_send![&*device, setBatteryMonitoringEnabled: true];
        let battery: f32 = msg_send![&*device, batteryLevel];
        (thermal, battery)
    };
    let buffers = gpu_buffers - window.gpu_buffers;
    let line = format!(
        "{{\"scene\":{:?},\"seconds\":{elapsed:.1},\"frames\":{},\"gap_p50_ms\":{:.2},\"gap_p95_ms\":{:.2},\"gap_p99_ms\":{:.2},\"gap_max_ms\":{:.2},\"cpu_pct\":{:.1},\"gpu_app_pct\":{:.2},\"gpu_ms_per_buffer\":{:.3},\"gpu_buffers\":{buffers},\"memory_mb\":{:.1},\"thermal\":{thermal},\"battery\":{battery:.3}}}",
        window.scene,
        gaps.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        at(1.),
        100. * (cpu - window.cpu) / elapsed,
        100. * (gpu_ns - window.gpu_ns) as f64 / 1e9 / elapsed,
        (gpu_ns - window.gpu_ns) as f64 / 1e6 / buffers.max(1) as f64,
        footprint_mb(),
    );
    println!("ZGX_METRICS {line}");
    let _ = std::io::stdout().flush();
    if let Some(log) = &mut window.log {
        let _ = writeln!(log, "{line}");
    }
    window.since = now;
    window.gaps_ms.clear();
    window.cpu = cpu;
    window.gpu_ns = gpu_ns;
    window.gpu_buffers = gpu_buffers;
}

/// Start, if `ZGX_IOS_METRICS` asks.
pub(crate) fn start() {
    let Some(seconds) = zgx::env::var("ZGX_IOS_METRICS")
        .ok()
        .and_then(|seconds| seconds.parse::<f64>().ok())
    else {
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let log = std::env::var_os("HOME").and_then(|home| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::path::Path::new(&home).join("Documents/zgx-metrics.jsonl"))
            .ok()
    });
    let (gpu_ns, gpu_buffers) = zgx_gpu::metal_gpu_time();
    let sampler = Sampler::alloc(mtm).set_ivars(RefCell::new(Window {
        since: Instant::now(),
        last_tick: None,
        gaps_ms: Vec::new(),
        cpu: cpu_seconds(),
        gpu_ns,
        gpu_buffers,
        every: Duration::from_secs_f64(seconds.max(0.5)),
        scene: zgx::env::var("ZGX_IOS_SCENE").unwrap_or_default(),
        log,
    }));
    // SAFETY: NSObject's `init`, CADisplayLink's constructor and method; the
    // run loop keeps the link, and the link its target, for the app's life.
    unsafe {
        let sampler: Retained<Sampler> = msg_send![super(sampler), init];
        let link: Retained<AnyObject> = msg_send![class!(CADisplayLink),
            displayLinkWithTarget: &*sampler,
            selector: sel!(tick:)];
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: NSRunLoopCommonModes];
        std::mem::forget(sampler);
    }
}
