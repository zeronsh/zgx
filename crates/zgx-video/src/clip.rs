//! Video files shown in a video (`video.clip`): each is decoded by ffmpeg at
//! the composition's rate and scaled to its box (by AVFoundation on iOS,
//! which runs no ffmpeg: `clip_avf`), and each frame of the video shows
//! exactly the frame of the file its time says, decoded before the frame is
//! drawn. Played on, a decoder streams; asked for a frame behind it, it
//! starts again from there.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Child;
#[cfg(not(target_os = "ios"))]
use std::process::{Command, Stdio};

/// A clip a script has placed.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// The name its picture is shown by (`host:<id>`).
    pub id: String,
    /// Its file.
    pub src: PathBuf,
    /// The composition's frame it starts at.
    pub from: f64,
    /// Frames it shows for; `None`, as long as the file runs.
    pub duration: Option<f64>,
    /// Frames of the file skipped.
    pub trim: f64,
    /// Its size in pixels.
    pub size: (u32, u32),
    /// It plays again from its start when it ends.
    pub looped: bool,
}

/// One clip's decoder.
pub(crate) struct Decoder {
    clip: Clip,
    fps: f64,
    child: Option<Stream>,
    /// The frame the pipe gives next.
    next: u64,
    /// The file has no more frames: its last is held.
    ended: Option<u64>,
    last: Option<(u64, Vec<u8>)>,
}
impl Decoder {
    pub(crate) fn new(clip: Clip, fps: f64) -> Self {
        Self {
            clip,
            fps,
            child: None,
            next: 0,
            ended: None,
            last: None,
        }
    }
    pub(crate) fn clip(&self) -> &Clip {
        &self.clip
    }
    /// The file's frame `frame` at the composition's rate, RGBA; the last one
    /// there is past its end. An error if ffmpeg can't read it.
    pub(crate) fn frame(&mut self, mut frame: u64) -> Result<&[u8], String> {
        if let Some(end) = self.ended {
            frame = if self.clip.looped && end > 0 {
                frame % end
            } else {
                frame.min(end.saturating_sub(1))
            };
        }
        if self.last.as_ref().is_some_and(|(at, _)| *at == frame) {
            return Ok(&self.last.as_ref().expect("checked").1);
        }
        if self.child.is_none() || frame < self.next || frame > self.next + 120 {
            self.start(frame)?;
        }
        let (width, height) = self.clip.size;
        let mut pixels = self
            .last
            .take()
            .map(|(_, pixels)| pixels)
            .unwrap_or_default();
        pixels.resize(width as usize * height as usize * 4, 0);
        while self.next <= frame {
            match self.child.as_mut().map(|stream| stream.read(&mut pixels)) {
                Some(true) => self.next += 1,
                _ => {
                    // Past its end: the file has `next` frames.
                    self.child = None;
                    if self.next == 0 {
                        return Err(format!("{}: no frames", self.clip.src.display()));
                    }
                    self.ended = Some(self.next);
                    return self.frame(frame);
                }
            }
        }
        self.last = Some((frame, pixels));
        Ok(&self.last.as_ref().expect("set").1)
    }
    #[cfg(target_os = "ios")]
    fn start(&mut self, frame: u64) -> Result<(), String> {
        self.stop();
        let at = frame as f64 / self.fps;
        let reader = crate::clip_avf::Reader::new(&self.clip.src, at, self.fps, self.clip.size)?;
        self.child = Some(Stream::Avf(Box::new(reader)));
        self.next = frame;
        Ok(())
    }
    #[cfg(not(target_os = "ios"))]
    fn start(&mut self, frame: u64) -> Result<(), String> {
        self.stop();
        let ffmpeg = crate::encode::ffmpeg().ok_or_else(|| {
            format!(
                "video.clip needs ffmpeg to read {}, and found none on the PATH",
                self.clip.src.display()
            )
        })?;
        let (width, height) = self.clip.size;
        let at = frame as f64 / self.fps;
        let child = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-ss", &format!("{at:.6}")])
            .arg("-i")
            .arg(&self.clip.src)
            .args(["-an", "-vf"])
            .arg(format!(
                "fps={},scale={width}:{height}:flags=lanczos,format=rgba",
                self.fps
            ))
            .args(["-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("couldn't run ffmpeg: {error}"))?;
        self.child = Some(Stream::Ffmpeg(child));
        self.next = frame;
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(Stream::Ffmpeg(mut child)) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Where a decoder's frames come from: ffmpeg's pipe, or AVFoundation's reader.
enum Stream {
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    Ffmpeg(Child),
    #[cfg(target_os = "ios")]
    Avf(Box<crate::clip_avf::Reader>),
}
impl Stream {
    /// The next frame into `pixels`, or false at the file's end.
    fn read(&mut self, pixels: &mut [u8]) -> bool {
        match self {
            Self::Ffmpeg(child) => child
                .stdout
                .as_mut()
                .is_some_and(|out| out.read_exact(pixels).is_ok()),
            #[cfg(target_os = "ios")]
            Self::Avf(reader) => reader.read(pixels),
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A video file's size in pixels, as shown (by AVFoundation on iOS).
#[cfg(target_os = "ios")]
pub(crate) fn size_of(src: &Path) -> Option<(u32, u32)> {
    crate::clip_avf::size_of(src)
}
/// Whether a file has a sound to mix.
#[cfg(target_os = "ios")]
pub(crate) fn has_audio(src: &Path) -> bool {
    crate::clip_avf::has_audio(src)
}

/// A video file's size in pixels, by ffprobe beside ffmpeg.
#[cfg(not(target_os = "ios"))]
pub(crate) fn size_of(src: &Path) -> Option<(u32, u32)> {
    let ffprobe =
        crate::encode::ffmpeg()?.with_file_name(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=width,height", "-of", "csv=p=0"])
        .arg(src)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text.trim().split_once(',')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Whether a file has a sound to mix.
#[cfg(not(target_os = "ios"))]
pub(crate) fn has_audio(src: &Path) -> bool {
    let Some(ffprobe) = crate::encode::ffmpeg()
        .map(|ffmpeg| ffmpeg.with_file_name(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)))
    else {
        return false;
    };
    Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "a", "-show_entries"])
        .args(["stream=index", "-of", "csv=p=0"])
        .arg(src)
        .output()
        .is_ok_and(|out| !out.stdout.is_empty())
}
