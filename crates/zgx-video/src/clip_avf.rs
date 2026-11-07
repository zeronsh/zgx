//! A clip's frames by AVFoundation, where no ffmpeg can run (iOS starts no
//! processes): `AVAssetReader` decodes the file from a time, and each read
//! gives the next frame at the composition's rate, as ffmpeg's pipe does, so
//! `clip::Decoder` seeks, streams and ends the same way on both. A frame is the
//! file's frame on screen at its time, turned as the file says (a phone's
//! portrait recording, as ffmpeg turns it) and scaled to the clip's box.
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2::{Encode, Encoding, RefEncode, class, msg_send};
use objc2_core_foundation::CGSize;
use std::ffi::c_void;
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
// SAFETY: CoreMedia's `CMTime`, an anonymous struct of these fields.
unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[
            Encoding::LongLong,
            Encoding::Int,
            Encoding::UInt,
            Encoding::LongLong,
        ],
    );
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CMTimeRange {
    start: CMTime,
    duration: CMTime,
}
// SAFETY: CoreMedia's `CMTimeRange`, two `CMTime`s.
unsafe impl Encode for CMTimeRange {
    const ENCODING: Encoding = Encoding::Struct("?", &[CMTime::ENCODING, CMTime::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}
// SAFETY: CoreGraphics's `CGAffineTransform`, six doubles.
unsafe impl Encode for CGAffineTransform {
    const ENCODING: Encoding = Encoding::Struct(
        "CGAffineTransform",
        &[
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
        ],
    );
}
unsafe impl RefEncode for CGAffineTransform {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// CoreMedia's sample buffer, known only by pointer.
#[repr(C)]
struct OpaqueSampleBuffer {
    _private: [u8; 0],
}
// SAFETY: `CMSampleBufferRef`, a pointer to `struct opaqueCMSampleBuffer`.
unsafe impl RefEncode for OpaqueSampleBuffer {
    const ENCODING_REF: Encoding =
        Encoding::Pointer(&Encoding::Struct("opaqueCMSampleBuffer", &[]));
}

/// `kCMTimeFlags_Valid`.
const VALID: u32 = 1;
/// `kCVPixelFormatType_32BGRA`.
const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: *const AnyObject;
    static AVMediaTypeAudio: *const AnyObject;
}
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMTimePositiveInfinity: CMTime;
    fn CMTimeMakeWithSeconds(seconds: f64, timescale: i32) -> CMTime;
    fn CMTimeGetSeconds(time: CMTime) -> f64;
    fn CMSampleBufferGetPresentationTimeStamp(sample: *mut c_void) -> CMTime;
    fn CMSampleBufferGetImageBuffer(sample: *mut c_void) -> *mut c_void;
}
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: *const AnyObject;
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *const u8;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *mut c_void);
}
/// `kCVPixelBufferLock_ReadOnly`.
const READ_ONLY: u64 = 1;

/// A file's asset and its first track of `kind`.
fn track(src: &Path, video: bool) -> Option<(Retained<AnyObject>, Retained<AnyObject>)> {
    let path = objc2_foundation::NSString::from_str(src.to_str()?);
    // SAFETY: AVFoundation's documented class methods, with live arguments;
    // a track list's object is a live `AVAssetTrack`.
    unsafe {
        let url: Option<Retained<AnyObject>> = msg_send![class!(NSURL), fileURLWithPath: &*path];
        let asset: Option<Retained<AnyObject>> = msg_send![
            class!(AVURLAsset),
            URLAssetWithURL: &*url?,
            options: std::ptr::null::<AnyObject>()
        ];
        let asset = asset?;
        let kind = if video {
            AVMediaTypeVideo
        } else {
            AVMediaTypeAudio
        };
        let tracks: Option<Retained<AnyObject>> = msg_send![&*asset, tracksWithMediaType: kind];
        let first: Option<Retained<AnyObject>> = msg_send![&*tracks?, firstObject];
        Some((asset, first?))
    }
}

/// A turn of the picture as shown: where a shown pixel's centre comes from in
/// the file's, and the shown size.
#[derive(Clone, Copy, Debug)]
struct Turn {
    /// The inverse of the track's transform, its result moved to start at 0.
    inverse: [f64; 6],
    shown: (f64, f64),
}
impl Turn {
    fn of(t: CGAffineTransform, natural: (f64, f64)) -> Self {
        let (w, h) = natural;
        let at = |x: f64, y: f64| (t.a * x + t.c * y + t.tx, t.b * x + t.d * y + t.ty);
        let corners = [at(0., 0.), at(w, 0.), at(0., h), at(w, h)];
        let min_x = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let min_y = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
        let max_x = corners
            .iter()
            .map(|c| c.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = corners
            .iter()
            .map(|c| c.1)
            .fold(f64::NEG_INFINITY, f64::max);
        // Shown = file · M + (tx − min); file = (shown − offset) · M⁻¹.
        let det = t.a * t.d - t.b * t.c;
        let det = if det.abs() < 1e-9 { 1. } else { det };
        let (ox, oy) = (t.tx - min_x, t.ty - min_y);
        let (ia, ib, ic, id) = (t.d / det, -t.b / det, -t.c / det, t.a / det);
        Self {
            inverse: [ia, ib, ic, id, -(ox * ia + oy * ic), -(ox * ib + oy * id)],
            shown: ((max_x - min_x).round(), (max_y - min_y).round()),
        }
    }
    fn source(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, tx, ty] = self.inverse;
        (a * x + c * y + tx, b * x + d * y + ty)
    }
}

/// A video file's size as shown (turned as it says), in pixels.
pub(crate) fn size_of(src: &Path) -> Option<(u32, u32)> {
    autoreleasepool(|_| {
        let (_, track) = track(src, true)?;
        // SAFETY: an `AVAssetTrack`'s properties.
        let (size, transform): (CGSize, CGAffineTransform) = unsafe {
            (
                msg_send![&*track, naturalSize],
                msg_send![&*track, preferredTransform],
            )
        };
        let turn = Turn::of(transform, (size.width, size.height));
        Some((turn.shown.0 as u32, turn.shown.1 as u32))
    })
}

/// Whether a file has a sound.
pub(crate) fn has_audio(src: &Path) -> bool {
    autoreleasepool(|_| track(src, false).is_some())
}

/// A decoded frame of the file: its time and its BGRA rows.
struct Sample {
    at: f64,
    width: usize,
    height: usize,
    stride: usize,
    bgra: Vec<u8>,
}

/// A clip's frames from one time on, at a rate, scaled to a size.
pub(crate) struct Reader {
    _asset: Retained<AnyObject>,
    reader: Retained<AnyObject>,
    output: Retained<AnyObject>,
    turn: Turn,
    fps: f64,
    size: (u32, u32),
    /// The time asked to start at.
    at: f64,
    /// The time of the frame the next read gives (`None` before the first).
    next: Option<f64>,
    /// The frame shown at `next`'s time, and the one after it.
    shown: Option<Sample>,
    ahead: Option<Sample>,
    /// The file has no frames after `ahead`.
    drained: bool,
    /// How long a file's frame shows: past its last one's time by this, the
    /// file has ended.
    frame_time: f64,
}
impl Reader {
    /// Frames from `at` seconds, at `fps`, each `size` pixels.
    pub(crate) fn new(src: &Path, at: f64, fps: f64, size: (u32, u32)) -> Result<Self, String> {
        autoreleasepool(|_| {
            let missing = || format!("{}: no video track AVFoundation reads", src.display());
            let (asset, track) = track(src, true).ok_or_else(missing)?;
            // SAFETY: AVFoundation's documented calls on live objects: an
            // asset reader of this asset, a track output of its track in BGRA,
            // and its time range set before it starts reading.
            unsafe {
                let size_natural: CGSize = msg_send![&*track, naturalSize];
                let transform: CGAffineTransform = msg_send![&*track, preferredTransform];
                let rate: f32 = msg_send![&*track, nominalFrameRate];
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let reader: Option<Retained<AnyObject>> = msg_send![
                    class!(AVAssetReader),
                    assetReaderWithAsset: &*asset,
                    error: &mut error
                ];
                let reader =
                    reader.ok_or_else(|| format!("{}: AVAssetReader refused it", src.display()))?;
                let format: Retained<AnyObject> =
                    msg_send![class!(NSNumber), numberWithUnsignedInt: BGRA];
                let settings: Retained<AnyObject> = msg_send![
                    class!(NSDictionary),
                    dictionaryWithObject: &*format,
                    forKey: kCVPixelBufferPixelFormatTypeKey
                ];
                let output: Option<Retained<AnyObject>> = msg_send![
                    class!(AVAssetReaderTrackOutput),
                    assetReaderTrackOutputWithTrack: &*track,
                    outputSettings: &*settings
                ];
                let output = output.ok_or_else(missing)?;
                let _: () = msg_send![&*output, setAlwaysCopiesSampleData: false];
                let can: bool = msg_send![&*reader, canAddOutput: &*output];
                if !can {
                    return Err(missing());
                }
                let _: () = msg_send![&*reader, addOutput: &*output];
                // From a frame before `at`, so the frame shown at `at` is read.
                let frame_time = if rate > 0. {
                    1. / rate as f64
                } else {
                    1. / 30.
                };
                let from = (at - frame_time).max(0.);
                let range = CMTimeRange {
                    start: CMTimeMakeWithSeconds(from, 600_000),
                    duration: kCMTimePositiveInfinity,
                };
                let _: () = msg_send![&*reader, setTimeRange: range];
                let started: bool = msg_send![&*reader, startReading];
                if !started {
                    return Err(format!("{}: AVAssetReader couldn't start", src.display()));
                }
                Ok(Self {
                    _asset: asset,
                    reader,
                    output,
                    turn: Turn::of(transform, (size_natural.width, size_natural.height)),
                    fps,
                    size,
                    at,
                    next: None,
                    shown: None,
                    ahead: None,
                    drained: false,
                    frame_time,
                })
            }
        })
    }
    /// The file's next frame, or none at its end.
    fn pull(&mut self) -> Option<Sample> {
        autoreleasepool(|_| {
            // SAFETY: `copyNextSampleBuffer` returns a sample buffer we own
            // (released below) or null; its image buffer is locked while read.
            unsafe {
                let sample: *mut OpaqueSampleBuffer =
                    msg_send![&*self.output, copyNextSampleBuffer];
                let sample = sample.cast::<c_void>();
                if sample.is_null() {
                    return None;
                }
                let time = CMSampleBufferGetPresentationTimeStamp(sample);
                let image = CMSampleBufferGetImageBuffer(sample);
                let mut out = None;
                if !image.is_null() && CVPixelBufferLockBaseAddress(image, READ_ONLY) == 0 {
                    let (width, height) =
                        (CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image));
                    let stride = CVPixelBufferGetBytesPerRow(image);
                    let base = CVPixelBufferGetBaseAddress(image);
                    if !base.is_null() {
                        out = Some(Sample {
                            at: if time.flags & VALID != 0 {
                                CMTimeGetSeconds(time)
                            } else {
                                0.
                            },
                            width,
                            height,
                            stride,
                            bgra: std::slice::from_raw_parts(base, stride * height).to_vec(),
                        });
                    }
                    CVPixelBufferUnlockBaseAddress(image, READ_ONLY);
                }
                CFRelease(sample);
                out
            }
        })
    }
    /// The next frame at the rate into `pixels` (RGBA, the clip's size), or
    /// false past the file's end.
    pub(crate) fn read(&mut self, pixels: &mut [u8]) -> bool {
        // The file's frame on screen at the output frame's time: the latest
        // that starts by then (a hair early counts, as times round); past
        // the last one's end, none.
        let next = self.next.unwrap_or(self.at);
        let by = next + 1e-6;
        if self.ahead.is_none() && !self.drained {
            self.ahead = self.pull();
            self.drained = self.ahead.is_none();
        }
        while self.ahead.as_ref().is_some_and(|s| s.at <= by) {
            self.shown = self.ahead.take();
            self.ahead = self.pull();
            self.drained = self.ahead.is_none();
        }
        let ended = self.ahead.is_none()
            && self
                .shown
                .as_ref()
                .is_none_or(|last| by >= last.at + self.frame_time);
        if ended {
            return false;
        }
        let Some(sample) = self.shown.as_ref().or(self.ahead.as_ref()) else {
            return false;
        };
        draw(sample, &self.turn, self.size, pixels);
        self.next = Some(next + 1. / self.fps);
        true
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: stopping a reader that may still be reading.
        unsafe {
            let _: () = msg_send![&*self.reader, cancelReading];
        }
    }
}

/// A frame turned as shown and scaled to `size` (bilinear), RGBA, opaque.
fn draw(sample: &Sample, turn: &Turn, size: (u32, u32), pixels: &mut [u8]) {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let (sx, sy) = (turn.shown.0 / w as f64, turn.shown.1 / h as f64);
    let max_x = sample.width.saturating_sub(1) as f64;
    let max_y = sample.height.saturating_sub(1) as f64;
    let at = |x: usize, y: usize, c: usize| sample.bgra[y * sample.stride + x * 4 + c] as f64;
    for oy in 0..h {
        for ox in 0..w {
            let (fx, fy) = turn.source((ox as f64 + 0.5) * sx, (oy as f64 + 0.5) * sy);
            let (fx, fy) = ((fx - 0.5).clamp(0., max_x), (fy - 0.5).clamp(0., max_y));
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = (
                (x0 + 1).min(sample.width - 1),
                (y0 + 1).min(sample.height - 1),
            );
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            let out = &mut pixels[(oy * w + ox) * 4..][..4];
            // BGRA in, RGBA out.
            for (to, from) in [(0, 2), (1, 1), (2, 0)] {
                let top = at(x0, y0, from) * (1. - tx) + at(x1, y0, from) * tx;
                let bottom = at(x0, y1, from) * (1. - tx) + at(x1, y1, from) * tx;
                out[to] = (top * (1. - ty) + bottom * ty).round() as u8;
            }
            out[3] = 255;
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    //! Against ffmpeg's own frames: the same frame of the file at each time,
    //! turned as ffmpeg turns it.
    use super::*;
    use std::process::Command;

    fn ffmpeg() -> Option<std::path::PathBuf> {
        crate::encode::ffmpeg()
    }
    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zgx-avf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    /// A clip of `frames` frames at `rate`, frame i a grey of 8·i, `w`×`h`,
    /// its left quarter white (to see a turn by), shown turned by `rotate`.
    fn clip(
        dir: &Path,
        rate: u32,
        frames: u32,
        (w, h): (u32, u32),
        rotate: i32,
    ) -> std::path::PathBuf {
        let (plain, out) = (
            dir.join(format!("p{rotate}.mp4")),
            dir.join(format!("c{rotate}.mp4")),
        );
        let filter = "geq=lum='if(lt(X,W/4),255,8*N)':cb=128:cr=128,format=yuv420p";
        let run = |args: &[&str], with: &[&std::ffi::OsStr]| {
            Command::new(ffmpeg().unwrap())
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(args)
                .args(with)
                .status()
                .is_ok_and(|s| s.success())
        };
        let source = format!(
            "color=black:s={w}x{h}:r={rate}:d={}",
            frames as f64 / rate as f64
        );
        let made = run(
            &[
                "-f", "lavfi", "-i", &source, "-vf", filter, "-c:v", "libx264", "-crf", "1", "-g",
                "4",
            ],
            &[plain.as_os_str()],
        );
        assert!(made, "ffmpeg made the clip");
        // Shown turned, as a phone's recording says (an input option).
        let turned = run(
            &["-display_rotation", &rotate.to_string(), "-i"],
            &[
                plain.as_os_str(),
                "-c".as_ref(),
                "copy".as_ref(),
                out.as_os_str(),
            ],
        );
        assert!(turned, "ffmpeg turned the clip");
        out
    }
    fn grey(pixels: &[u8], w: u32, h: u32) -> u8 {
        // The middle of the right half: away from the white quarter.
        let (x, y) = (w * 3 / 4, h / 2);
        pixels[((y * w + x) * 4) as usize]
    }

    /// ffmpeg's frames, as `clip::Decoder` reads them on the desktop.
    fn ffmpeg_frames(src: &Path, at: f64, fps: f64, (w, h): (u32, u32)) -> Vec<Vec<u8>> {
        let out = Command::new(ffmpeg().unwrap())
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-ss", &format!("{at:.6}"), "-i"])
            .arg(src)
            .args(["-an", "-vf"])
            .arg(format!("fps={fps},scale={w}:{h}:flags=lanczos,format=rgba"))
            .args(["-f", "rawvideo", "-"])
            .output()
            .unwrap()
            .stdout;
        out.chunks((w * h * 4) as usize)
            .map(<[u8]>::to_vec)
            .collect()
    }

    #[test]
    fn each_frame_is_the_files_frame_on_screen_at_its_time() {
        if ffmpeg().is_none() {
            return;
        }
        let dir = dir("frames");
        let src = clip(&dir, 24, 24, (64, 48), 0);
        assert_eq!(size_of(&src), Some((64, 48)));
        assert!(!has_audio(&src));
        // Each of the file's 24 frames as ffmpeg decodes it, at the file's rate.
        let file: Vec<u8> = ffmpeg_frames(&src, 0., 24., (64, 48))
            .iter()
            .map(|p| grey(p, 64, 48))
            .collect();
        assert_eq!(file.len(), 24);
        for (at, fps, (w, h)) in [
            (0.25, 30., (64, 48)),
            (0., 24., (64, 48)),
            (0.5, 12., (64, 48)),
            (0.1, 30., (32, 24)),
        ] {
            let mut reader = Reader::new(&src, at, fps, (w, h)).unwrap();
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let mut seen = Vec::new();
            while reader.read(&mut pixels) {
                seen.push(grey(&pixels, w, h));
            }
            // Output frame k at `at + k/fps` shows the file's frame floor(t·24).
            let want: Vec<u8> = (0..)
                .map(|k| ((at + k as f64 / fps) * 24. + 1e-6).floor() as usize)
                .take_while(|&i| i < 24)
                .map(|i| file[i])
                .collect();
            let case = format!("from {at} at {fps} in {w}x{h}");
            assert_eq!(seen.len(), want.len(), "{case}: {seen:?} against {want:?}");
            for (got, want_one) in seen.iter().zip(&want) {
                assert!(
                    got.abs_diff(*want_one) <= 3,
                    "{case}: {seen:?} against {want:?}"
                );
            }
            // ffmpeg's pipe (the desktop's decoder) rounds times its own way:
            // within one of the file's frames of it, frame by frame, from a
            // start on one of the file's frames. (From between two, ffmpeg's
            // seek drops the frame on screen then and starts a frame late.)
            if ((at * 24.) - (at * 24.).round()).abs() > 1e-6 {
                continue;
            }
            let piped: Vec<u8> = ffmpeg_frames(&src, at, fps, (w, h))
                .iter()
                .map(|p| grey(p, w, h))
                .collect();
            assert!(
                seen.len().abs_diff(piped.len()) <= 1,
                "{case}: {seen:?} against ffmpeg's {piped:?}"
            );
            for (got, theirs) in seen.iter().zip(&piped) {
                assert!(
                    got.abs_diff(*theirs) <= 12,
                    "{case}: {seen:?} against ffmpeg's {piped:?}"
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_turned_recording_shows_turned_and_its_size_as_shown() {
        if ffmpeg().is_none() {
            return;
        }
        let dir = dir("turn");
        for rotate in [90, -90, 180] {
            let src = clip(&dir, 10, 3, (64, 32), rotate);
            let (w, h) = if rotate == 180 { (64, 32) } else { (32, 64) };
            assert_eq!(size_of(&src), Some((w, h)), "rotate {rotate}");
            // ffmpeg's own turned frame, to compare with.
            let want = Command::new(ffmpeg().unwrap())
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(&src)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "format=rgba",
                    "-f",
                    "rawvideo",
                    "-",
                ])
                .output()
                .unwrap()
                .stdout;
            assert_eq!(want.len(), (w * h * 4) as usize, "ffmpeg turned it too");
            let mut reader = Reader::new(&src, 0., 10., (w, h)).unwrap();
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            assert!(reader.read(&mut pixels));
            // Where the white quarter went: the same pixels white in both.
            let white = |p: &[u8]| -> Vec<bool> { p.chunks(4).map(|c| c[0] > 200).collect() };
            let (a, b) = (white(&pixels), white(&want));
            let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
            assert!(
                differ <= (w + h) as usize,
                "rotate {rotate}: {differ} pixels differ"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
