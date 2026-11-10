//! The stock effects: each an [`Effect`](crate::Effect) to put on a bus
//! with [`Bus::add_effect`](crate::Bus::add_effect), its parameters set by
//! the index constants on its type.
//!
//! ```
//! use zgx_audio::effects::{LowPass, Slope};
//! use zgx_audio::{Audio, Config, Ramp};
//!
//! let (audio, _mixer) = Audio::offline(Config::default())?;
//! let music = audio.bus("music").unwrap();
//! let muffle = music.add_effect(LowPass::new(18_000.).slope(Slope::Db24))?;
//! muffle.set(LowPass::CUTOFF, 400., Ramp::ms(300.))?; // under water
//! # Ok::<(), zgx_audio::Error>(())
//! ```
mod delay;
mod dynamics;
#[cfg(feature = "hrtf")]
mod fft;
mod filter;
#[cfg(feature = "hrtf")]
mod hrtf;
mod reverb;

pub use delay::Delay;
pub use dynamics::{Compressor, Duck};
pub use filter::{FLAT, HighPass, LowPass, Peaking, Slope};
#[cfg(feature = "hrtf")]
pub use hrtf::{BLOCK, Binaural, HrtfSet};
pub use reverb::Reverb;
