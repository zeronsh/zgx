//! Sound for zgx apps: a mixer that places every start, stop and change on
//! an exact frame, buses with gains, mutes, solos and fades, a look-ahead
//! limiter, and WAV files, rendered offline on a clock of its own (this is
//! what tests compare) or, later, to a device.
//!
//! ```
//! use zgx_audio::{Audio, Config, Play, Ramp, Sound};
//!
//! let (audio, mut mixer) = Audio::offline(Config::default());
//! let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.06).sin() * 0.5).collect();
//! let tone = Sound::from_samples(&tone, 1, 48_000)?;
//! let ui = audio.bus("ui").unwrap();
//! let voice = audio.play(&tone, Play::new().bus(&ui).at(audio.now() + audio.seconds(0.01)))?;
//! voice.set_gain(0.25, Ramp::ms(20.))?;
//! let out = mixer.frames(4800);
//! assert!(out[..480 * 2].iter().all(|&s| s == 0.), "nothing before its frame");
//! # Ok::<(), zgx_audio::Error>(())
//! ```
//!
//! Threads: the [`Audio`] handle and every [`Voice`] and [`Bus`] are the
//! main thread's (or any thread's); the mixer runs on the audio thread and
//! never waits for them. Sounds it has finished with are dropped by the
//! next call on [`Audio`] (or [`Audio::update`]), never by the mixer.

mod decode;
mod device;
mod engine;
mod limiter;
mod mixer;
mod queue;
mod ramp;
mod resample;
mod ring;
mod sound;
mod stream;
#[cfg(feature = "device")]
mod via_cpal;
pub mod wav;

pub use device::{Device, Format, Renderer};
pub use engine::{Audio, Bus, Config, Error, Offline, Play, Stats, Time, Voice};
pub use mixer::Timing;
pub use ramp::{Curve, Ramp};
pub use resample::Interpolation;
pub use sound::{Sound, Source};
#[cfg(feature = "device")]
pub use via_cpal::{Cpal, Stopping};
