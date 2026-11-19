//! H.264, HEVC and ProRes 4444 written by macOS itself (AVFoundation's
//! `AVAssetWriter`, on VideoToolbox's hardware), with no ffmpeg: frames go
//! into pixel buffers from the writer's own pool. Video only, for now: a
//! render with sound is written by ffmpeg.
//!
//! As ffmpeg writes them (`encode::Encoder::ffmpeg`): Rec. 709 primaries,
//! transfer and matrix, limited range, 4:2:0 for H.264 and HEVC; ProRes 4444
//! with its alpha; the index at the front of an MP4 (`faststart`). Only even sides: 4:2:0 needs
//! them, and ffmpeg's path trims an odd side where this one would refuse.
//!
//! What this can't write, it says before the first frame (`start`) or as
//! the first frame goes in, and `encode` writes with ffmpeg instead: a file
//! is never worse for being written here.
use crate::encode::{EncodeOptions, Format};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor, AVAssetWriterStatus,
    AVFileTypeMPEG4, AVFileTypeQuickTimeMovie, AVMediaTypeVideo, AVVideoCodecKey,
    AVVideoCodecTypeAppleProRes4444, AVVideoCodecTypeH264, AVVideoCodecTypeHEVC,
    AVVideoColorPrimaries_ITU_R_709_2, AVVideoColorPrimariesKey, AVVideoColorPropertiesKey,
    AVVideoCompressionPropertiesKey, AVVideoHeightKey, AVVideoQualityKey,
    AVVideoTransferFunction_ITU_R_709_2, AVVideoTransferFunctionKey, AVVideoWidthKey,
    AVVideoYCbCrMatrix_ITU_R_709_2, AVVideoYCbCrMatrixKey,
};
use objc2_core_media::CMTime;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferPool,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelBufferWidthKey, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::time::{Duration, Instant};

/// A file macOS is writing.
pub(crate) struct Writer {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
    adaptor: Retained<AVAssetWriterInputPixelBufferAdaptor>,
    size: (u32, u32),
    /// Frames at `index * 1000 / timescale` seconds: exact for 29.97 too.
    timescale: i32,
}
// SAFETY: the writer and its inputs are used from one thread at a time (the
// encode thread, after `start` on the caller's), as AVFoundation allows.
unsafe impl Send for Writer {}

/// Whether this path writes `format` (it may still refuse at `start`).
pub(crate) fn writes(format: Format) -> bool {
    matches!(format, Format::H264 | Format::Hevc | Format::ProRes)
}

/// `quality` (lower better, on libx264's CRF scale; libx265's is it plus 4)
/// as VideoToolbox's `AVVideoQualityKey` (0 to 1, higher better) for
/// `format`: between the measured points, a straight line (`H264`, `HEVC`).
fn quality_of(format: Format, quality: u32) -> f64 {
    let table = match format {
        Format::Hevc => HEVC,
        _ => H264,
    };
    let crf = quality as f64;
    let (first, last) = (table[0], table[table.len() - 1]);
    if crf <= first.0 {
        return first.1;
    }
    if crf >= last.0 {
        return last.1;
    }
    let at = table
        .windows(2)
        .find(|w| crf <= w[1].0)
        .expect("within the table");
    let (a, b) = (at[0], at[1]);
    a.1 + (b.1 - a.1) * (crf - a.0) / (b.0 - a.0)
}
/// For each quality on libx264's scale, the VideoToolbox setting whose PSNR
/// matches libx264's there. Measured on the showcase's first 300 frames
/// (1920×1080, 60 fps), against a lossless libx264 of the same Rec. 709 4:2:0
/// frames, on an Apple M5 Max with macOS 26.4 (Nov 2026):
///
/// | libx264 CRF | PSNR | size | VideoToolbox q | PSNR | size |
/// | --- | --- | --- | --- | --- | --- |
/// | 16 | 54.03 dB | 2,192 KB | 0.90 | 53.14 dB | 8,060 KB |
/// | 20 | 52.14 dB | 1,388 KB | 0.75 | 52.08 dB | 1,918 KB |
/// | 24 | 49.95 dB | 889 KB | 0.60 | 49.62 dB | 947 KB |
/// | 28 | 47.63 dB | 614 KB | 0.50 | 46.80 dB | 625 KB |
///
/// It tops out near 53.5 dB (0.95: 53.48 dB in 26 MB), below libx264's
/// default: from 18 down, 0.9, past which it grows far more than it gains.
const H264: &[(f64, f64)] = &[(18., 0.9), (20., 0.76), (24., 0.62), (28., 0.53)];
/// As `H264`, for libx265 (its CRF the quality plus 4), measured the same way:
///
/// | quality (libx265 CRF) | PSNR | size |
/// | --- | --- | --- |
/// | 16 (20) | 52.90 dB | 622 KB |
/// | 20 (24) | 51.04 dB | 429 KB |
/// | 24 (28) | 48.86 dB | 313 KB |
/// | 28 (32) | 46.21 dB | 229 KB |
///
/// | VideoToolbox q | 0.50 | 0.60 | 0.70 | 0.85 | 0.90 |
/// | --- | --- | --- | --- | --- | --- |
/// | PSNR | 47.84 dB | 50.28 dB | 51.90 dB | 52.85 dB | 53.21 dB |
/// | size | 416 KB | 622 KB | 999 KB | 2,512 KB | 5,449 KB |
///
/// Never below the measured 0.5, which already passes 28's quality.
const HEVC: &[(f64, f64)] = &[(16., 0.85), (20., 0.65), (24., 0.55), (28., 0.5)];

fn key(key: Option<&'static NSString>) -> Result<&'static NSString, String> {
    key.ok_or_else(|| "AVFoundation's settings keys are missing".into())
}
fn dictionary(pairs: &[(&NSString, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let (keys, values): (Vec<&NSString>, Vec<&AnyObject>) = pairs.iter().copied().unzip();
    NSDictionary::from_slices(&keys, &values)
}

impl Writer {
    /// Ready to take frames, or why not (before any file is made).
    pub(crate) fn start(options: &EncodeOptions) -> Result<Self, String> {
        let (width, height) = options.size;
        if options.audio.is_some() {
            return Err("sound, which only ffmpeg adds yet".into());
        }
        if width % 2 != 0 || height % 2 != 0 {
            return Err("an odd side, which 4:2:0 can't hold".into());
        }
        // SAFETY: AVFoundation's documented constants, set at load.
        let (codec, file_type) = unsafe {
            match options.format {
                Format::H264 => (AVVideoCodecTypeH264, AVFileTypeMPEG4),
                Format::Hevc => (AVVideoCodecTypeHEVC, AVFileTypeMPEG4),
                Format::ProRes => (AVVideoCodecTypeAppleProRes4444, AVFileTypeQuickTimeMovie),
                _ => return Err("not a format macOS writes here".into()),
            }
        };
        let (Some(codec), Some(file_type)) = (codec, file_type) else {
            return Err("this macOS names no such codec".into());
        };
        // The writer won't write over a file; ffmpeg's `-y` would.
        let _ = std::fs::remove_file(&options.output);
        let path = NSString::from_str(&options.output.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        // SAFETY: a file URL and a declared file type.
        let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&url, file_type) }
            .map_err(|error| error.localizedDescription().to_string())?;
        // SAFETY: AVFoundation's constants, and settings of the types its
        // documentation gives: numbers, strings and dictionaries.
        let input = unsafe {
            let size = |n: u32| NSNumber::new_u32(n);
            let colour = dictionary(&[
                (
                    key(AVVideoColorPrimariesKey)?,
                    key(AVVideoColorPrimaries_ITU_R_709_2)?,
                ),
                (
                    key(AVVideoTransferFunctionKey)?,
                    key(AVVideoTransferFunction_ITU_R_709_2)?,
                ),
                (
                    key(AVVideoYCbCrMatrixKey)?,
                    key(AVVideoYCbCrMatrix_ITU_R_709_2)?,
                ),
            ]);
            let (w, h) = (size(width), size(height));
            let mut pairs: Vec<(&NSString, &AnyObject)> = vec![
                (key(AVVideoCodecKey)?, codec),
                (key(AVVideoWidthKey)?, &w),
                (key(AVVideoHeightKey)?, &h),
                (key(AVVideoColorPropertiesKey)?, &colour),
            ];
            let quality =
                NSNumber::new_f64(quality_of(options.format, options.quality.unwrap_or(16)));
            let compression = dictionary(&[(key(AVVideoQualityKey)?, &quality)]);
            if options.format != Format::ProRes {
                pairs.push((key(AVVideoCompressionPropertiesKey)?, &compression));
            }
            let settings = dictionary(&pairs);
            let video = AVMediaTypeVideo.ok_or("no video media type")?;
            if !writer.canApplyOutputSettings_forMediaType(Some(&settings), video) {
                return Err("macOS can't write these settings".into());
            }
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(video, Some(&settings))
        };
        // SAFETY: as above; CoreVideo's own keys, as CFStrings toll-free
        // bridged to the NSStrings a dictionary of attributes takes.
        let adaptor = unsafe {
            input.setExpectsMediaDataInRealTime(false);
            let (w, h) = (NSNumber::new_u32(width), NSNumber::new_u32(height));
            let bgra = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let as_ns = |k: &objc2_core_foundation::CFString| -> &NSString {
                &*(k as *const objc2_core_foundation::CFString).cast::<NSString>()
            };
            let attributes = dictionary(&[
                (as_ns(kCVPixelBufferPixelFormatTypeKey), &bgra),
                (as_ns(kCVPixelBufferWidthKey), &w),
                (as_ns(kCVPixelBufferHeightKey), &h),
            ]);
            AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(
                &input,
                Some(&attributes),
            )
        };
        // SAFETY: an input made for this writer, added before writing starts.
        unsafe {
            if !writer.canAddInput(&input) {
                return Err("macOS won't add this video track".into());
            }
            writer.addInput(&input);
        }
        // SAFETY: inputs added; nothing written yet.
        unsafe {
            writer.setShouldOptimizeForNetworkUse(options.format != Format::ProRes);
            if !writer.startWriting() {
                return Err(failure(&writer, "macOS wouldn't start writing"));
            }
            writer.startSessionAtSourceTime(CMTime::new(0, 1));
        }
        let timescale = (options.fps * 1000.).round();
        if !(1. ..=i32::MAX as f64).contains(&timescale) {
            return Err("a frame rate this path can't time".into());
        }
        Ok(Self {
            writer,
            input,
            adaptor,
            size: (width, height),
            timescale: timescale as i32,
        })
    }
    /// Append frame `index`: straight RGBA, `size` by `size`.
    pub(crate) fn write(&mut self, frame: &[u8], index: usize) -> Result<(), String> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        if frame.len() != width * height * 4 {
            return Err("a frame of the wrong size".into());
        }
        let at = index as i64 * 1000;
        self.wait_ready()?;
        let buffer = self.buffer()?;
        // SAFETY: a buffer of `size` from the pool, BGRA, locked while its
        // rows (`bytes per row` apart, at least `width` pixels) are written.
        unsafe {
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags(0)) != 0 {
                return Err("a pixel buffer didn't lock".into());
            }
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let row = CVPixelBufferGetBytesPerRow(&buffer);
            if base.is_null() {
                CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags(0));
                return Err("a pixel buffer has no pixels".into());
            }
            for (y, source) in frame.chunks_exact(width * 4).enumerate() {
                let line = std::slice::from_raw_parts_mut(base.add(y * row), width * 4);
                for (to, from) in line
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(source.as_chunks::<4>().0)
                {
                    *to = [from[2], from[1], from[0], from[3]];
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags(0));
            if !self
                .adaptor
                .appendPixelBuffer_withPresentationTime(&buffer, CMTime::new(at, self.timescale))
            {
                return Err(failure(&self.writer, "macOS refused a frame"));
            }
        }
        Ok(())
    }
    /// All frames in: the file.
    pub(crate) fn finish(self) -> Result<(), String> {
        // SAFETY: every frame appended; the writer finishes on its own
        // queue and says so through the block, waited for here.
        unsafe {
            self.input.markAsFinished();
            let (done, finished) = std::sync::mpsc::channel();
            let block = block2::RcBlock::new(move || {
                let _ = done.send(());
            });
            self.writer.finishWritingWithCompletionHandler(&block);
            finished
                .recv_timeout(Duration::from_secs(600))
                .map_err(|_| "macOS never finished the file".to_owned())?;
            if self.writer.status() != AVAssetWriterStatus::Completed {
                return Err(failure(&self.writer, "macOS couldn't finish the file"));
            }
        }
        Ok(())
    }
    /// Gives up the file (a frame refused, or ffmpeg writing it instead).
    pub(crate) fn cancel(self) {
        // SAFETY: cancelling a writer that started; it removes its file.
        unsafe { self.writer.cancelWriting() };
    }
    fn buffer(&self) -> Result<objc2_core_foundation::CFRetained<CVPixelBuffer>, String> {
        // SAFETY: the adaptor's pool, there once writing started; a
        // successful Create returns a +1 reference, adopted.
        unsafe {
            let pool = self
                .adaptor
                .pixelBufferPool()
                .ok_or("no pixel buffer pool")?;
            let mut buffer = std::ptr::null_mut();
            let status = CVPixelBufferPool::create_pixel_buffer(
                None,
                &pool,
                std::ptr::NonNull::from(&mut buffer),
            );
            let buffer = std::ptr::NonNull::new(buffer).filter(|_| status == 0);
            buffer
                .map(|buffer| objc2_core_foundation::CFRetained::from_raw(buffer))
                .ok_or_else(|| format!("no pixel buffer: {status}"))
        }
    }
    /// Waits while the frames' track can take no more, as the encoder catches up.
    fn wait_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            // SAFETY: a plain read of the input's state.
            if unsafe { self.input.isReadyForMoreMediaData() } {
                return Ok(());
            }
            // SAFETY: a plain read of the writer's state.
            if unsafe { self.writer.status() } == AVAssetWriterStatus::Failed {
                return Err(failure(&self.writer, "macOS stopped writing"));
            }
            if Instant::now() > deadline {
                return Err("macOS stopped taking frames".into());
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }
}

/// `what`, and what the writer says went wrong.
fn failure(writer: &AVAssetWriter, what: &str) -> String {
    // SAFETY: a plain read of the writer's error.
    match unsafe { writer.error() } {
        Some(error) => format!("{what}: {}", error.localizedDescription()),
        None => what.to_owned(),
    }
}
