//! Luau in the browser (wasm32, under zgx-dom): its C++ is built against
//! wasi-libc (`crates/zgx-dom/luau-env.sh`), whose system calls are answered
//! here, in the module (a browser has no WASI host): no environment, no
//! files, stdio writes dropped, the clock the browser's. And C++'s static
//! constructors (Luau's flags) run once, before the first VM: a library on
//! wasm32-unknown-unknown has no start function that would run them.
use crate::time::{Instant, SystemTime};
use std::sync::{Once, OnceLock};

unsafe extern "C" {
    fn __wasm_call_ctors();
}

/// Run C++'s static constructors, once.
pub(crate) fn start() {
    static STARTED: Once = Once::new();
    // SAFETY: called once, before any Luau code runs.
    STARTED.call_once(|| unsafe { __wasm_call_ctors() });
}

/// When the module started, for the monotonic clock.
fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// libc's `clock()`, for `os.clock`: microseconds (wasi-libc's
/// `CLOCKS_PER_SEC`) since the module started.
#[unsafe(no_mangle)]
extern "C" fn clock() -> i64 {
    origin().elapsed().as_micros() as i64
}

/// WASI's errors, as wasi-libc reads them.
const BADF: i32 = 8;
const SPIPE: i32 = 70;

#[unsafe(no_mangle)]
extern "C" fn __imported_wasi_snapshot_preview1_environ_get(_: i32, _: i32) -> i32 {
    0
}
/// No environment: no variables, no bytes.
///
/// # Safety
/// wasi-libc passes two writable `u32`s.
#[unsafe(no_mangle)]
unsafe extern "C" fn __imported_wasi_snapshot_preview1_environ_sizes_get(
    count: *mut u32,
    size: *mut u32,
) -> i32 {
    // SAFETY: as the caller promises.
    unsafe {
        *count = 0;
        *size = 0;
    }
    0
}
/// Nanoseconds: wall time for the realtime clock (0), else since start.
///
/// # Safety
/// wasi-libc passes a writable `u64`.
#[unsafe(no_mangle)]
unsafe extern "C" fn __imported_wasi_snapshot_preview1_clock_time_get(
    id: i32,
    _: i64,
    out: *mut u64,
) -> i32 {
    let nanos = if id == 0 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    } else {
        origin().elapsed().as_nanos() as u64
    };
    // SAFETY: as the caller promises.
    unsafe { *out = nanos };
    0
}
#[unsafe(no_mangle)]
extern "C" fn __imported_wasi_snapshot_preview1_fd_close(_: i32) -> i32 {
    BADF
}
#[unsafe(no_mangle)]
extern "C" fn __imported_wasi_snapshot_preview1_fd_fdstat_get(_: i32, _: i32) -> i32 {
    BADF
}
#[unsafe(no_mangle)]
extern "C" fn __imported_wasi_snapshot_preview1_fd_seek(_: i32, _: i64, _: i32, _: i32) -> i32 {
    SPIPE
}
/// Writes to stdout and stderr, dropped (all of them written).
///
/// # Safety
/// wasi-libc passes `n` (pointer, length) pairs and a writable `u32`.
#[unsafe(no_mangle)]
unsafe extern "C" fn __imported_wasi_snapshot_preview1_fd_write(
    _: i32,
    iovs: *const [u32; 2],
    n: usize,
    written: *mut u32,
) -> i32 {
    // SAFETY: as the caller promises.
    unsafe {
        *written = (0..n).map(|i| (*iovs.add(i))[1]).sum();
    }
    0
}
#[unsafe(no_mangle)]
extern "C" fn __imported_wasi_snapshot_preview1_proc_exit(_: i32) -> ! {
    std::process::abort()
}
