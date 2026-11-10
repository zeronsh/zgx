//! Sounds decoded whole and shared by every voice that plays them.
use crate::Error;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
    pub fn from_samples(samples: &[f32], channels: usize, rate: u32) -> Result<Sound, Error> {
        if channels == 0 || rate == 0 || !samples.len().is_multiple_of(channels) {
            return Err(Error::Format(format!(
                "{} samples can't be frames of {channels} channels at {rate} Hz",
                samples.len()
            )));
        }
        let samples: Box<[f32]> = match channels {
            1 | 2 => samples.into(),
            _ => samples
                .chunks_exact(channels)
                .flat_map(|frame| [frame[0], frame[1]])
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
        #[cfg(feature = "vorbis")]
        if bytes.starts_with(b"OggS") {
            let (samples, channels, rate) =
                crate::decode::vorbis::decode_whole(self.vorbis_bytes(Some(bytes.into_owned())))
                    .map_err(named)?;
            return Sound::from_samples(&samples, channels, rate);
        }
        Err(named(Error::Format(Self::READS.into())))
    }

    const READS: &'static str = if cfg!(feature = "vorbis") {
        "not a format zgx-audio reads (WAV, Ogg Vorbis)"
    } else {
        "not a format zgx-audio reads (WAV; Ogg Vorbis with the `vorbis` feature)"
    };

    /// Where a Vorbis decoder reads from: bytes already read, the bytes in
    /// memory, or the file again.
    #[cfg(feature = "vorbis")]
    fn vorbis_bytes(&self, read: Option<Vec<u8>>) -> crate::decode::vorbis::Bytes {
        use crate::decode::vorbis::Bytes;
        match (self, read) {
            (_, Some(read)) => Bytes::Memory(read.into()),
            (Source::Bytes(Cow::Borrowed(bytes)), None) => Bytes::Static(bytes),
            (Source::Bytes(Cow::Owned(bytes)), None) => Bytes::Memory(bytes.clone().into()),
            (Source::File(path), None) => Bytes::File(path.clone()),
        }
    }

    /// A decoder for this source, for a stream: the source's own for Ogg
    /// Vorbis; WAV decoded whole and read out in pieces.
    pub(crate) fn decoder(&self) -> Result<Box<dyn crate::decode::Decode>, Error> {
        let head = match self {
            Source::Bytes(bytes) => bytes.get(..4).map(|h| h.to_vec()).unwrap_or_default(),
            Source::File(path) => {
                use std::io::Read;
                let mut head = [0u8; 4];
                let mut file = std::fs::File::open(path)
                    .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
                let got = file.read(&mut head).unwrap_or(0);
                head[..got].to_vec()
            }
        };
        #[cfg(feature = "vorbis")]
        if head == b"OggS" {
            let vorbis = crate::decode::vorbis::Vorbis::new(self.vorbis_bytes(None));
            return match self {
                Source::File(path) => vorbis.map_err(|e| e.within(&path.display().to_string())),
                Source::Bytes(_) => vorbis,
            }
            .map(|v| Box::new(v) as Box<dyn crate::decode::Decode>);
        }
        let _ = head;
        let sound = self.decode()?;
        Ok(Box::new(crate::decode::Pcm::new(sound.0)))
    }
}
