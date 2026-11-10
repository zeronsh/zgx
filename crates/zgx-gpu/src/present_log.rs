//! `ZGX_PRESENT_LOG=1`: a line on stderr for each presentation event (the
//! surface a frame used, layer hand-offs, trims, focus), stamped with the
//! host clock (`CLOCK_UPTIME_RAW`, as ScreenCaptureKit and Core Media stamp
//! frames; on Windows the performance counter, as DXGI and DWM stamp them;
//! on Linux `CLOCK_MONOTONIC`, as Wayland's presentation feedback stamps
//! them), to line a screen recording's frames up with what happened.
use std::sync::OnceLock;

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| zgx::env::var_os("ZGX_PRESENT_LOG").is_some_and(|v| v != "0"))
}

/// Layer-state polling is a separate observer from timestamped event logs.
#[cfg(target_os = "macos")]
pub(crate) fn sample_layers() -> bool {
    enabled() && zgx::env::var_os("ZGX_PRESENT_LAYERS").is_some_and(|v| v != "0")
}

/// Nanoseconds on the host clock.
pub fn now_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: as `<time.h>` declares it (`clockid_t` is a 32-bit enum).
        unsafe extern "C" {
            fn clock_gettime_nsec_np(clock: u32) -> u64;
        }
        // SAFETY: takes a clock id and returns its time, 0 for an unknown
        // clock; 8 is CLOCK_UPTIME_RAW.
        unsafe { clock_gettime_nsec_np(8) }
    }
    #[cfg(windows)]
    {
        crate::dxgi::now_ns()
    }
    // 64-bit Linux, whose `timespec` is two 64-bit fields.
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    {
        #[repr(C)]
        struct Timespec {
            sec: i64,
            nsec: i64,
        }
        // SAFETY: as `<time.h>` declares it (`clockid_t` is an int).
        unsafe extern "C" {
            fn clock_gettime(clock: i32, out: *mut Timespec) -> i32;
        }
        let mut t = Timespec { sec: 0, nsec: 0 };
        // SAFETY: CLOCK_MONOTONIC (1), into a timespec of the size it writes.
        if unsafe { clock_gettime(1, &mut t) } != 0 {
            return 0;
        }
        t.sec as u64 * 1_000_000_000 + t.nsec as u64
    }
    #[cfg(not(any(
        target_os = "macos",
        windows,
        all(target_os = "linux", target_pointer_width = "64")
    )))]
    {
        0
    }
}

/// Logs `what` when `ZGX_PRESENT_LOG` is set; costs one load otherwise.
pub fn log(what: impl FnOnce() -> String) {
    if enabled() {
        eprintln!("PRESENT {:.6} {}", now_ns() as f64 / 1e9, what());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn layer_sampling_requires_both_switches() {
        const EXPECTED: &str = "ZGX_TEST_PRESENT_LAYERS_EXPECTED";
        if let Ok(expected) = std::env::var(EXPECTED) {
            assert_eq!(super::sample_layers(), expected == "1");
            return;
        }
        // Each process reads the real switches and its own cached log flag.
        for (log, layers, expected) in [
            (None, None, false),
            (Some("1"), None, false),
            (Some("1"), Some("0"), false),
            (Some("0"), Some("1"), false),
            (Some("1"), Some("1"), true),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "present_log::tests::layer_sampling_requires_both_switches",
            ]);
            for key in [
                "ZGX_PRESENT_LOG",
                "ZGUI_PRESENT_LOG",
                "ZGX_PRESENT_LAYERS",
                "ZGUI_PRESENT_LAYERS",
            ] {
                child.env_remove(key);
            }
            if let Some(value) = log {
                child.env("ZGX_PRESENT_LOG", value);
            }
            if let Some(value) = layers {
                child.env("ZGX_PRESENT_LAYERS", value);
            }
            let output = child
                .env(EXPECTED, if expected { "1" } else { "0" })
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "log={log:?}, layers={layers:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
