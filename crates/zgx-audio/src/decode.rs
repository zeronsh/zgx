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

/// AAC (in MP4/M4A, or raw ADTS), through the platform's own decoder:
/// AudioToolbox on macOS. No other platform has one here yet: there the
/// feature builds, and an AAC source is an error that says so.
#[cfg(feature = "aac")]
pub(crate) mod aac {
    use crate::Error;

    /// Whether `head` (a source's first bytes) is AAC this reads: an MP4
    /// (its `ftyp` box) or an ADTS stream (its 12-bit sync word, layer 0).
    pub(crate) fn is_aac(head: &[u8]) -> bool {
        head.get(4..8) == Some(b"ftyp")
            || matches!(head, [0xff, second, ..] if second & 0xf6 == 0xf0)
    }

    #[cfg(target_os = "macos")]
    pub(crate) use platform::Aac;

    #[cfg(not(target_os = "macos"))]
    pub(crate) struct Aac;

    #[cfg(not(target_os = "macos"))]
    impl Aac {
        pub(crate) fn new(_: std::sync::Arc<[u8]>) -> Result<Aac, Error> {
            Err(Error::Format(
                "AAC, which zgx-audio reads with the platform's decoder: macOS's only, so far"
                    .into(),
            ))
        }
    }

    #[cfg(not(target_os = "macos"))]
    impl super::Decode for Aac {
        fn channels(&self) -> usize {
            2
        }
        fn rate(&self) -> u32 {
            48_000
        }
        fn next(&mut self, _: &mut Vec<f32>) -> Result<bool, Error> {
            Ok(false)
        }
        fn rewind(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    /// The whole of `bytes`: its samples, channels and rate.
    pub(crate) fn decode_whole(
        bytes: std::sync::Arc<[u8]>,
    ) -> Result<(Vec<f32>, usize, u32), Error> {
        use super::Decode;
        let mut aac = Aac::new(bytes)?;
        let mut samples = Vec::new();
        while aac.next(&mut samples)? {}
        Ok((samples, aac.channels(), aac.rate()))
    }

    /// AudioToolbox: an `AudioFile` read through callbacks from the bytes
    /// in memory, wrapped in an `ExtAudioFile` that decodes to the client
    /// format asked of it (32-bit floats, interleaved), trimming the
    /// encoder's priming and padding where the file says how much.
    #[cfg(target_os = "macos")]
    mod platform {
        use super::super::{Decode, PIECE};
        use crate::Error;
        use std::ffi::c_void;
        use std::sync::Arc;

        type OsStatus = i32;
        type AudioFileId = *mut c_void;
        type ExtAudioFileRef = *mut c_void;

        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct StreamDescription {
            sample_rate: f64,
            format: u32,
            flags: u32,
            bytes_per_packet: u32,
            frames_per_packet: u32,
            bytes_per_frame: u32,
            channels: u32,
            bits: u32,
            reserved: u32,
        }

        #[repr(C)]
        struct Buffer {
            channels: u32,
            bytes: u32,
            data: *mut c_void,
        }

        #[repr(C)]
        struct BufferList {
            count: u32,
            buffers: [Buffer; 1],
        }

        type ReadProc = extern "C" fn(*mut c_void, i64, u32, *mut c_void, *mut u32) -> OsStatus;
        type SizeProc = extern "C" fn(*mut c_void) -> i64;

        #[link(name = "AudioToolbox", kind = "framework")]
        unsafe extern "C" {
            fn AudioFileOpenWithCallbacks(
                client: *mut c_void,
                read: ReadProc,
                write: *const c_void,
                size: SizeProc,
                set_size: *const c_void,
                hint: u32,
                file: *mut AudioFileId,
            ) -> OsStatus;
            fn AudioFileClose(file: AudioFileId) -> OsStatus;
            fn ExtAudioFileWrapAudioFileID(
                file: AudioFileId,
                writing: u8,
                ext: *mut ExtAudioFileRef,
            ) -> OsStatus;
            fn ExtAudioFileDispose(ext: ExtAudioFileRef) -> OsStatus;
            fn ExtAudioFileGetProperty(
                ext: ExtAudioFileRef,
                id: u32,
                size: *mut u32,
                data: *mut c_void,
            ) -> OsStatus;
            fn ExtAudioFileSetProperty(
                ext: ExtAudioFileRef,
                id: u32,
                size: u32,
                data: *const c_void,
            ) -> OsStatus;
            fn ExtAudioFileRead(
                ext: ExtAudioFileRef,
                frames: *mut u32,
                data: *mut BufferList,
            ) -> OsStatus;
            fn ExtAudioFileSeek(ext: ExtAudioFileRef, frame: i64) -> OsStatus;
        }

        const fn code(c: &[u8; 4]) -> u32 {
            u32::from_be_bytes(*c)
        }

        /// The bytes the callbacks read: boxed, so their address holds
        /// while the file reads them.
        struct Held(Arc<[u8]>);

        extern "C" fn read(
            client: *mut c_void,
            at: i64,
            asked: u32,
            into: *mut c_void,
            got: *mut u32,
        ) -> OsStatus {
            // SAFETY: `client` is the `Held` this `Aac` boxed, alive while
            // its file is; `into` has room for `asked` bytes, and `got` is
            // the count the caller reads.
            unsafe {
                let bytes = &(&*(client as *const Held)).0;
                let start = usize::try_from(at).unwrap_or(usize::MAX).min(bytes.len());
                let n = (asked as usize).min(bytes.len() - start);
                std::ptr::copy_nonoverlapping(bytes[start..].as_ptr(), into as *mut u8, n);
                *got = n as u32;
            }
            0
        }

        extern "C" fn size(client: *mut c_void) -> i64 {
            // SAFETY: as for `read`.
            let held = unsafe { &*(client as *const Held) };
            held.0.len() as i64
        }

        /// An AAC source being decoded.
        pub(crate) struct Aac {
            held: Box<Held>,
            file: AudioFileId,
            ext: ExtAudioFileRef,
            channels: usize,
            rate: u32,
            /// What each read decodes into: aligned to 64 bytes, always,
            /// so the decoder takes the same path for every buffer, and two
            /// decodes of one file come out the same to the last bit.
            piece: Box<Piece>,
        }

        #[repr(C, align(64))]
        struct Piece([f32; PIECE * 2]);

        // SAFETY: the file and its converter are used from one thread at a
        // time (the decode thread's, or the caller's), never two at once;
        // AudioToolbox objects aren't tied to the thread that made them.
        unsafe impl Send for Aac {}

        fn failed(what: &str, status: OsStatus) -> Error {
            Error::Format(format!("AAC the platform couldn't {what} ({status})"))
        }

        impl Aac {
            pub(crate) fn new(bytes: Arc<[u8]>) -> Result<Aac, Error> {
                let hint = if bytes.get(4..8) == Some(b"ftyp") {
                    code(b"m4af")
                } else {
                    code(b"adts")
                };
                let held = Box::new(Held(bytes));
                let client = &*held as *const Held as *mut c_void;
                let mut file: AudioFileId = std::ptr::null_mut();
                // SAFETY: the callbacks read `held`, which outlives the
                // file (closed in `Drop` before the box goes).
                let status = unsafe {
                    AudioFileOpenWithCallbacks(
                        client,
                        read,
                        std::ptr::null(),
                        size,
                        std::ptr::null(),
                        hint,
                        &mut file,
                    )
                };
                if status != 0 {
                    return Err(failed("open", status));
                }
                let mut ext: ExtAudioFileRef = std::ptr::null_mut();
                // SAFETY: `file` is open; on failure it's closed here.
                let status = unsafe { ExtAudioFileWrapAudioFileID(file, 0, &mut ext) };
                if status != 0 {
                    unsafe { AudioFileClose(file) };
                    return Err(failed("decode", status));
                }
                let mut aac = Aac {
                    held,
                    file,
                    ext,
                    channels: 0,
                    rate: 0,
                    piece: Box::new(Piece([0.; PIECE * 2])),
                };
                let mut source = StreamDescription::default();
                let mut size = size_of::<StreamDescription>() as u32;
                // SAFETY: `source` has room for what's asked.
                let status = unsafe {
                    ExtAudioFileGetProperty(
                        aac.ext,
                        code(b"ffmt"),
                        &mut size,
                        &mut source as *mut _ as *mut c_void,
                    )
                };
                if status != 0 {
                    return Err(failed("read the format of", status));
                }
                let channels = source.channels.clamp(1, 2);
                let rate = source.sample_rate.round() as u32;
                if !(1_000..=768_000).contains(&rate) {
                    return Err(Error::Format(format!("AAC at {rate} Hz")));
                }
                // Floats, interleaved, one or two channels, at its own rate.
                let client = StreamDescription {
                    sample_rate: source.sample_rate,
                    format: code(b"lpcm"),
                    // Float, packed.
                    flags: 1 | 8,
                    bytes_per_packet: 4 * channels,
                    frames_per_packet: 1,
                    bytes_per_frame: 4 * channels,
                    channels,
                    bits: 32,
                    reserved: 0,
                };
                // SAFETY: `client` is a whole description.
                let status = unsafe {
                    ExtAudioFileSetProperty(
                        aac.ext,
                        code(b"cfmt"),
                        size_of::<StreamDescription>() as u32,
                        &client as *const _ as *const c_void,
                    )
                };
                if status != 0 {
                    return Err(failed("convert", status));
                }
                aac.channels = channels as usize;
                aac.rate = rate;
                Ok(aac)
            }
        }

        impl Decode for Aac {
            fn channels(&self) -> usize {
                self.channels
            }
            fn rate(&self) -> u32 {
                self.rate
            }
            fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
                let mut frames = PIECE as u32;
                let mut list = BufferList {
                    count: 1,
                    buffers: [Buffer {
                        channels: self.channels as u32,
                        bytes: (PIECE * self.channels * 4) as u32,
                        data: self.piece.0.as_mut_ptr() as *mut c_void,
                    }],
                };
                // SAFETY: the buffer is `piece`, room for `PIECE` frames.
                let status = unsafe { ExtAudioFileRead(self.ext, &mut frames, &mut list) };
                if status != 0 {
                    return Err(failed("decode", status));
                }
                if frames == 0 {
                    return Ok(false);
                }
                out.extend_from_slice(&self.piece.0[..frames as usize * self.channels]);
                Ok(true)
            }
            fn rewind(&mut self) -> Result<(), Error> {
                // SAFETY: the converter is open.
                match unsafe { ExtAudioFileSeek(self.ext, 0) } {
                    0 => Ok(()),
                    status => Err(failed("go back to the start of", status)),
                }
            }
        }

        impl Drop for Aac {
            fn drop(&mut self) {
                // SAFETY: both open, each closed once, before `held` goes.
                unsafe {
                    ExtAudioFileDispose(self.ext);
                    AudioFileClose(self.file);
                }
                let _ = &self.held;
            }
        }
    }
}

/// MP3 (and MPEG Layer I and II), through nanomp3: pure Rust, gapless where
/// the file's LAME or Info tag says its delay and padding.
#[cfg(feature = "mp3")]
pub(crate) mod mp3 {
    use super::{Decode, PIECE};
    use crate::Error;
    use std::io::Cursor;
    use std::sync::Arc;

    /// Whether `head` (a source's first bytes) is MPEG audio: an ID3 tag,
    /// or a frame's 11-bit sync word with a layer (ADTS, AAC's, has none).
    pub(crate) fn is_mp3(head: &[u8]) -> bool {
        head.starts_with(b"ID3")
            || matches!(head, [0xff, second, ..] if second & 0xe0 == 0xe0 && second & 0x06 != 0)
    }

    pub(crate) struct Mp3 {
        reader: nanomp3::Reader<Cursor<Arc<[u8]>>, f32>,
        channels: usize,
        rate: u32,
        piece: Vec<f32>,
    }

    fn failed(error: impl std::fmt::Display) -> Error {
        Error::Format(format!("MP3 that can't be decoded: {error}"))
    }

    impl Mp3 {
        pub(crate) fn new(bytes: Arc<[u8]>) -> Result<Mp3, Error> {
            let reader = nanomp3::Reader::new(Cursor::new(bytes)).map_err(failed)?;
            let channels = reader
                .channels()
                .ok_or_else(|| Error::Format("MP3 with no audio".into()))?
                .num() as usize;
            let rate = reader.sample_rate();
            if !(1_000..=768_000).contains(&rate) || !(1..=2).contains(&channels) {
                return Err(Error::Format(format!(
                    "MP3 of {channels} channels at {rate} Hz"
                )));
            }
            Ok(Mp3 {
                reader,
                channels,
                rate,
                piece: vec![0.; PIECE * channels],
            })
        }
    }

    impl Decode for Mp3 {
        fn channels(&self) -> usize {
            self.channels
        }
        fn rate(&self) -> u32 {
            self.rate
        }
        fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
            let n = self.reader.read(&mut self.piece).map_err(failed)?;
            out.extend_from_slice(&self.piece[..n]);
            Ok(n > 0)
        }
        fn rewind(&mut self) -> Result<(), Error> {
            self.reader.seek(0).map_err(failed)
        }
    }

    /// The whole of `bytes`: its samples, channels and rate.
    pub(crate) fn decode_whole(bytes: Arc<[u8]>) -> Result<(Vec<f32>, usize, u32), Error> {
        let mut mp3 = Mp3::new(bytes)?;
        let mut samples = Vec::new();
        while mp3.next(&mut samples)? {}
        Ok((samples, mp3.channels, mp3.rate))
    }
}

/// Ogg Opus, through opus-pure: pure Rust, at 48 kHz, the encoder's delay
/// (the pre-skip) and the end's padding taken off, the header's gain put on.
#[cfg(feature = "opus")]
pub(crate) mod opus {
    use super::Decode;
    use crate::Error;
    use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, OggPacket, OpusDecoder, Trim};
    use std::io::Cursor;
    use std::sync::Arc;

    /// Whether `head` is Ogg Opus: an Ogg page whose first packet is an
    /// `OpusHead` (a first page has one segment: the packet starts at 28).
    pub(crate) fn is_opus(head: &[u8]) -> bool {
        head.starts_with(b"OggS") && head.get(28..36) == Some(b"OpusHead")
    }

    pub(crate) struct Opus {
        bytes: Arc<[u8]>,
        reader: OggOpusReader<Cursor<Arc<[u8]>>>,
        decoder: OpusDecoder,
        trim: Trim,
        channels: usize,
        packet: OggPacket,
        block: Vec<f32>,
    }

    fn failed(error: impl std::fmt::Display) -> Error {
        Error::Format(format!("Ogg Opus that can't be decoded: {error}"))
    }

    impl Opus {
        pub(crate) fn new(bytes: Arc<[u8]>) -> Result<Opus, Error> {
            let reader = OggOpusReader::new(Cursor::new(bytes.clone())).map_err(failed)?;
            let channels = usize::from(reader.head().channel_count);
            if !(1..=2).contains(&channels) {
                return Err(Error::Format(format!(
                    "Ogg Opus of {channels} channels: zgx-audio plays one or two"
                )));
            }
            let decoder = reader.head().decoder(48_000).map_err(failed)?;
            let trim = Trim::new(reader.head(), 48_000, channels).map_err(failed)?;
            Ok(Opus {
                bytes,
                reader,
                decoder,
                trim,
                channels,
                packet: OggPacket::default(),
                block: vec![0.; MAX_PACKET_SAMPLES * channels],
            })
        }
    }

    impl Decode for Opus {
        fn channels(&self) -> usize {
            self.channels
        }
        fn rate(&self) -> u32 {
            48_000
        }
        fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, Error> {
            if !self
                .reader
                .read_packet_into(&mut self.packet)
                .map_err(failed)?
            {
                return Ok(false);
            }
            let n = self
                .decoder
                .decode(&self.packet.data, MAX_PACKET_SAMPLES, &mut self.block)
                .map_err(failed)?;
            out.extend_from_slice(
                self.trim
                    .keep(&self.packet, &self.block[..n * self.channels]),
            );
            Ok(true)
        }
        fn rewind(&mut self) -> Result<(), Error> {
            // Opened again: the reader, the decoder's state and the trim
            // all start over.
            *self = Opus::new(self.bytes.clone())?;
            Ok(())
        }
    }

    /// The whole of `bytes`: its samples, channels and rate.
    pub(crate) fn decode_whole(bytes: Arc<[u8]>) -> Result<(Vec<f32>, usize, u32), Error> {
        let mut opus = Opus::new(bytes)?;
        let mut samples = Vec::new();
        while opus.next(&mut samples)? {}
        Ok((samples, opus.channels, 48_000))
    }
}
