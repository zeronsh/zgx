//! Sounds decoded whole and shared by every voice that plays them.
use crate::Error;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The most frames a sound can have: 12 hours at 48 kHz. Longer is streamed.
const MAX_FRAMES: usize = 1 << 31;

/// The most a sample can be, either way: +18 dB over full scale.
pub const MAX_SAMPLE: f32 = 8.;

/// A sample as the mixer may rely on it: finite, and within ±[`MAX_SAMPLE`].
fn sample(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-MAX_SAMPLE, MAX_SAMPLE)
    } else {
        0.
    }
}

/// Decoded samples at the file's own rate: one channel or two, interleaved.
#[derive(Debug)]
pub(crate) struct SoundData {
    pub samples: Box<[f32]>,
    /// 1 or 2.
    pub channels: usize,
    pub rate: u32,
}

impl SoundData {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
}

/// A sound held in memory. Cloning it is cheap: the samples are shared.
#[derive(Clone, Debug)]
pub struct Sound(pub(crate) Arc<SoundData>);

impl Sound {
    /// Interleaved samples of `channels` channels at `rate` frames a second.
    /// Past two channels, the first two (front left and right) are kept.
    /// Samples are kept to ±[`MAX_SAMPLE`] (+18 dB over full scale), and any
    /// that isn't a number or is infinite is silence. The rate must be one
    /// an engine can run at (1 kHz to 768 kHz), and there can be at most
    /// 2^31 frames: a voice's place in it is a frame shifted 32 bits, with
    /// room for its step.
    pub fn from_samples(samples: &[f32], channels: usize, rate: u32) -> Result<Sound, Error> {
        if channels == 0
            || !(1_000..=768_000).contains(&rate)
            || !samples.len().is_multiple_of(channels)
            || samples.len() / channels > MAX_FRAMES
        {
            return Err(Error::Format(format!(
                "{} samples can't be frames of {channels} channels at {rate} Hz",
                samples.len()
            )));
        }
        let samples: Box<[f32]> = match channels {
            1 | 2 => samples.iter().map(|&s| sample(s)).collect(),
            _ => samples
                .chunks_exact(channels)
                .flat_map(|frame| [sample(frame[0]), sample(frame[1])])
                .collect(),
        };
        Ok(Sound(Arc::new(SoundData {
            samples,
            channels: channels.min(2),
            rate,
        })))
    }

    /// Frames in it.
    pub fn frames(&self) -> usize {
        self.0.frames()
    }
    /// 1 or 2.
    pub fn channels(&self) -> usize {
        self.0.channels
    }
    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.0.rate
    }
    /// How long it plays at its own rate.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.0.rate))
    }
}

/// Where a sound's bytes come from.
#[derive(Clone, Debug)]
pub enum Source {
    Bytes(Cow<'static, [u8]>),
    File(PathBuf),
}

impl Source {
    /// Bytes in memory, such as `include_bytes!("click.wav")`.
    pub fn bytes(bytes: impl Into<Cow<'static, [u8]>>) -> Source {
        Source::Bytes(bytes.into())
    }
    /// A file, read when loaded.
    pub fn file(path: impl Into<PathBuf>) -> Source {
        Source::File(path.into())
    }

    pub(crate) fn decode(&self) -> Result<Sound, Error> {
        let bytes = match self {
            Source::Bytes(bytes) => Cow::Borrowed(&**bytes),
            Source::File(path) => Cow::Owned(
                std::fs::read(path)
                    .map_err(|error| Error::Io(format!("{}: {error}", path.display())))?,
            ),
        };
        let named = |error: Error| match self {
            Source::File(path) => error.within(&path.display().to_string()),
            Source::Bytes(_) => error,
        };
        if bytes.starts_with(b"RIFF") {
            let wav = crate::wav::read(&bytes).map_err(named)?;
            return Sound::from_samples(&wav.samples, wav.channels, wav.rate);
        }
        Err(named(Error::Format(
            "not a format zgx-audio reads (WAV)".into(),
        )))
    }
}
