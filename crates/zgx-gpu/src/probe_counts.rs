//! (flush, local probe) What frames submit and present: counts, and the
//! calling thread's CPU time inside each call (`CLOCK_THREAD_CPUTIME_ID`).
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy)]
pub enum Kind {
    /// A command buffer committed by zgx's own Metal paths.
    Commit,
    /// A drawable presented (`present`, `presentDrawable:`).
    Present,
    /// A layer's contents set to an IOSurface.
    SetContents,
    /// `nextDrawable`.
    NextDrawable,
    /// A `CATransaction` committed by zgx.
    Transaction,
    /// A wait for the GPU (`waitUntilScheduled`/`Completed`).
    Wait,
    /// wgpu's `queue.submit` (all its command buffers).
    WgpuSubmit,
}
pub const KINDS: [&str; 7] = ["commit", "present", "set_contents", "next_drawable", "transaction", "wait", "wgpu_submit"];
static COUNTS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static NANOS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

fn thread_cpu_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn clock_gettime_nsec_np(clock: u32) -> u64;
        }
        // SAFETY: 16 is CLOCK_THREAD_CPUTIME_ID.
        unsafe { clock_gettime_nsec_np(16) }
    }
    #[cfg(not(target_os = "macos"))]
    0
}

pub fn timed<R>(kind: Kind, f: impl FnOnce() -> R) -> R {
    let start = thread_cpu_ns();
    let out = f();
    COUNTS[kind as usize].fetch_add(1, Relaxed);
    NANOS[kind as usize].fetch_add(thread_cpu_ns().saturating_sub(start), Relaxed);
    out
}

/// Counts and CPU nanoseconds since the last take.
pub fn take() -> ([u64; 7], [u64; 7]) {
    let mut counts = [0; 7];
    let mut nanos = [0; 7];
    for i in 0..7 {
        counts[i] = COUNTS[i].swap(0, Relaxed);
        nanos[i] = NANOS[i].swap(0, Relaxed);
    }
    (counts, nanos)
}

/// Bytes uploaded, by kind.
pub const BYTE_KINDS: [&str; 4] = ["vertex_ring", "new_buffer", "inline", "texture"];
static BYTES: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// `n` bytes of kind `kind` (an index into `BYTE_KINDS`) uploaded.
pub fn bytes(kind: usize, n: usize) {
    BYTES[kind].fetch_add(n as u64, Relaxed);
}
/// Bytes since the last take.
pub fn take_bytes() -> [u64; 4] {
    let mut out = [0; 4];
    for i in 0..4 {
        out[i] = BYTES[i].swap(0, Relaxed);
    }
    out
}

static REGIONS: std::sync::Mutex<Vec<(&'static str, u64, u64)>> = std::sync::Mutex::new(Vec::new());
/// The calling thread's CPU in `f`, added to region `name` (count, ns).
pub fn region<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    let start = thread_cpu_ns();
    let out = f();
    let spent = thread_cpu_ns().saturating_sub(start);
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    match regions.iter_mut().find(|(n, _, _)| *n == name) {
        Some(entry) => {
            entry.1 += 1;
            entry.2 += spent;
        }
        None => regions.push((name, 1, spent)),
    }
    out
}
/// Regions since the last take: name, calls, CPU ns.
pub fn take_regions() -> Vec<(&'static str, u64, u64)> {
    std::mem::take(&mut *REGIONS.lock().unwrap_or_else(|e| e.into_inner()))
}
/// This thread's CPU so far, ns.
pub fn thread_cpu() -> u64 {
    thread_cpu_ns()
}
