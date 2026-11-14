//! (flush, local probe) The calling thread's CPU in named regions.
#![allow(missing_docs)]
static REGIONS: std::sync::Mutex<Vec<(&'static str, u64, u64)>> = std::sync::Mutex::new(Vec::new());
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
pub fn take_regions() -> Vec<(&'static str, u64, u64)> {
    std::mem::take(&mut *REGIONS.lock().unwrap_or_else(|e| e.into_inner()))
}
