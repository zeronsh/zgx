//! Decoders a stream reads from: a piece at a time, on the decode thread.
use crate::Error;
use crate::sound::SoundData;
use std::sync::Arc;

/// A source of frames at its own rate: one or two channels, interleaved.
pub(crate) trait Decode: Send {
    fn channels(&self) -> usize;
    fn rate(&self) -> u32;
    /// Appends the next frames to `out`. `Ok(false)` at the end.
    fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error>;
    /// Back to the first frame, for a loop.
    fn rewind(&mut self) -> Result<(), Error>;
}

/// Frames already in memory (a WAV), read out in pieces.
pub(crate) struct Pcm {
    data: Arc<SoundData>,
    at: usize,
}

impl Pcm {
    pub fn new(data: Arc<SoundData>) -> Pcm {
        Pcm { data, at: 0 }
    }
}

/// Frames a decoder hands over at a time, about 20 ms at 48 kHz.
const PIECE: usize = 1024;

impl Decode for Pcm {
    fn channels(&self) -> usize {
        self.data.channels
    }
    fn rate(&self) -> u32 {
        self.data.rate
    }
    fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
        let samples = &self.data.samples;
        if self.at >= samples.len() {
            return Ok(false);
        }
        let end = (self.at + PIECE * self.data.channels).min(samples.len());
        out.extend_from_slice(&samples[self.at..end]);
        self.at = end;
        Ok(true)
    }
    fn rewind(&mut self) -> Result<(), Error> {
        self.at = 0;
        Ok(())
    }
}

/// Ogg Vorbis, through lewton.
#[cfg(feature = "vorbis")]
pub(crate) mod vorbis {
    use super::Decode;
    use crate::Error;
    use lewton::inside_ogg::OggStreamReader;
    use std::io::{BufReader, Cursor, Read, Seek};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Where the bytes are, so a loop can open them again.
    #[derive(Clone)]
    pub(crate) enum Bytes {
        Static(&'static [u8]),
        Memory(Arc<[u8]>),
        File(PathBuf),
    }

    trait Input: Read + Seek + Send {}
    impl<T: Read + Seek + Send> Input for T {}

    pub(crate) struct Vorbis {
        bytes: Bytes,
        reader: OggStreamReader<Box<dyn Input>>,
        channels: usize,
        rate: u32,
    }

    fn open(bytes: &Bytes) -> Result<OggStreamReader<Box<dyn Input>>, Error> {
        let input: Box<dyn Input> = match bytes {
            Bytes::Static(bytes) => Box::new(Cursor::new(*bytes)),
            Bytes::Memory(bytes) => Box::new(Cursor::new(bytes.clone())),
            Bytes::File(path) => Box::new(BufReader::new(
                std::fs::File::open(path)
                    .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?,
            )),
        };
        OggStreamReader::new(input).map_err(|e| Error::Format(format!("Ogg Vorbis: {e}")))
    }

    impl Vorbis {
        pub fn new(bytes: Bytes) -> Result<Vorbis, Error> {
            let reader = open(&bytes)?;
            let channels = usize::from(reader.ident_hdr.audio_channels);
            let rate = reader.ident_hdr.audio_sample_rate;
            // The rates an engine runs at: a stream's ring is sized by its
            // rate, so a header's absurd one mustn't size it.
            if channels == 0 || !(1_000..=768_000).contains(&rate) {
                return Err(Error::Format(format!(
                    "Ogg Vorbis of {channels} channels at {rate} Hz"
                )));
            }
            Ok(Vorbis {
                bytes,
                reader,
                channels,
                rate,
            })
        }
    }

    impl Decode for Vorbis {
        fn channels(&self) -> usize {
            self.channels.min(2)
        }
        fn rate(&self) -> u32 {
            self.rate
        }
        fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
            let packet = self
                .reader
                .read_dec_packet_itl()
                .map_err(|e| Error::Format(format!("Ogg Vorbis: {e}")))?;
            let Some(packet) = packet else {
                return Ok(false);
            };
            let to_float = |s: i16| f32::from(s) / 32_768.;
            if self.channels <= 2 {
                out.extend(packet.into_iter().map(to_float));
            } else {
                // Past two channels, the first two (front left and right).
                for frame in packet.chunks_exact(self.channels) {
                    out.extend([to_float(frame[0]), to_float(frame[1])]);
                }
            }
            Ok(true)
        }
        fn rewind(&mut self) -> Result<(), Error> {
            self.reader = open(&self.bytes)?;
            Ok(())
        }
    }

    /// Decodes all of `bytes`.
    pub(crate) fn decode_whole(bytes: Bytes) -> Result<(Vec<f32>, usize, u32), Error> {
        let mut vorbis = Vorbis::new(bytes)?;
        let mut samples = Vec::new();
        while vorbis.next(&mut samples)? {}
        Ok((samples, vorbis.channels(), vorbis.rate()))
    }
}
