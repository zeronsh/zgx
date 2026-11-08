//! The system's audio output, one module a platform. The prototype has
//! macOS and iOS; `docs/audio-design.md` plans Windows (WASAPI), Linux and
//! Android.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use apple::{Device, Output};

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod none {
    use crate::mixer::Mixer;

    /// No output here yet.
    pub struct Device;
    pub struct Output;
    impl Device {
        pub fn open() -> Result<Self, String> {
            Err("zgx-audio has no output on this platform yet".into())
        }
        pub fn rate(&self) -> u32 {
            48_000
        }
        pub fn start(self, _mixer: Mixer) -> Result<Output, String> {
            Ok(Output)
        }
    }
}
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub use none::{Device, Output};
