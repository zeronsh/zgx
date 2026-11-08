//! The system's audio output, one module a platform: AudioUnit on macOS and
//! iOS, WASAPI on Windows, ALSA (loaded when a sound first plays) on Linux.
//!
//! Each runs the mixer on the system's audio thread, and closes the device
//! after the mixer has been silent a while ([`Watch::idle`]): an app that
//! has stopped making sound keeps no audio thread awake. The next command
//! opens it again ([`Gate::wake`]).
use crate::mixer::Watch;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::{Device, Output};

#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub use wasapi::{Device, Output};

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
mod alsa;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios"))))]
pub use alsa::{Device, Output};

#[cfg(not(any(unix, windows)))]
mod none {
    use crate::mixer::{Mixer, Watch};

    /// No output here.
    pub struct Device;
    pub struct Output;
    impl Device {
        pub fn open() -> Result<Self, String> {
            Err("zgx-audio has no output on this platform".into())
        }
        pub fn rate(&self) -> u32 {
            48_000
        }
        pub fn start(self, _mixer: Mixer, _watch: Watch) -> Result<Output, String> {
            Ok(Output)
        }
    }
    impl Output {
        pub fn wake(&self) {}
        pub fn is_running(&self) -> bool {
            false
        }
    }
}
#[cfg(not(any(unix, windows)))]
pub use none::{Device, Output};

const RUNNING: u8 = 0;
const CLOSING: u8 = 1;
const CLOSED: u8 = 2;

/// Whether a device runs, shared by the app's thread (which wakes it) and
/// the thread that closes it. Closing goes `RUNNING` → `CLOSING` → `CLOSED`
/// and then checks for a command sent meanwhile; waking goes `CLOSED` →
/// `RUNNING`. Whichever sees the other first acts, so a sound sent as the
/// device closes isn't left waiting.
#[derive(Default)]
pub(crate) struct Gate {
    state: AtomicU8,
    quit: AtomicBool,
}

impl Gate {
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::SeqCst) == RUNNING
    }
    /// Begins closing, if it runs.
    pub fn begin_close(&self) -> bool {
        self.state
            .compare_exchange(RUNNING, CLOSING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    /// Closed; `true` if a command came meanwhile and it runs again.
    pub fn closed(&self, watch: &Watch) -> bool {
        self.state.store(CLOSED, Ordering::SeqCst);
        watch.pending() && self.reopen()
    }
    /// `CLOSED` → `RUNNING`; whether this call made the change.
    pub fn reopen(&self) -> bool {
        self.state
            .compare_exchange(CLOSED, RUNNING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    pub fn quit(&self) {
        self.quit.store(true, Ordering::SeqCst);
    }
    pub fn quitting(&self) -> bool {
        self.quit.load(Ordering::SeqCst)
    }
    /// On a device's own thread, closed: waits until it should run again
    /// (`true`) or quit (`false`).
    #[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
    pub fn wait(&self, watch: &Watch) -> bool {
        loop {
            if self.quitting() {
                return false;
            }
            if self.is_running() || (watch.pending() && self.reopen()) {
                return true;
            }
            std::thread::park();
        }
    }
}
