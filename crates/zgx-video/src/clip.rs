//! Video files shown in a video (`video.clip`): each is decoded by ffmpeg at
//! the composition's rate and scaled to its box, and each frame of the video
//! shows exactly the frame of the file its time says, decoded before the
//! frame is drawn. Played on, a decoder streams; asked for a frame behind
//! it, it starts again from there.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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
    child: Option<Child>,
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
            let out = self.child.as_mut().and_then(|child| child.stdout.as_mut());
            let read = out.map(|out| out.read_exact(&mut pixels));
            match read {
                Some(Ok(())) => self.next += 1,
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
        // The file's frame on screen at each of the composition's: the last
        // to start at or before its time. Seeking to `at` itself would drop
        // the one that started a moment before it (ffmpeg drops frames
        // timed before where it seeks), so it seeks a second early, keeps
        // the file's times (from its start), takes for each composition
        // frame the last file frame at or before it (`round=up`), and
        // leaves out what comes before frame `frame` (half a frame early,
        // whatever the printed time rounds to) by its time: `select`, as
        // `trim` keeps a frame that only ends past where it starts.
        let early = (at - 1.).max(0.);
        let first = (frame as f64 - 0.5).max(0.) / self.fps;
        let child = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(["-ss", &format!("{early:.6}"), "-copyts", "-start_at_zero"])
            .arg("-i")
            .arg(&self.clip.src)
            .args(["-an", "-vf"])
            .arg(format!(
                "fps={}:round=up,select='gte(t,{first:.6})',scale={width}:{height}:flags=lanczos,format=rgba",
                self.fps
            ))
            .args(["-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("couldn't run ffmpeg: {error}"))?;
        self.child = Some(child);
        self.next = frame;
        Ok(())
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A video file's size in pixels, by ffprobe beside ffmpeg.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A 7 fps file of 30 frames (4.3 s), each its own grey: frame `k` is
    /// 8 + 8k. None without ffmpeg (and libx264rgb).
    fn greys(dir: &Path) -> Option<PathBuf> {
        let ffmpeg = crate::encode::ffmpeg()?;
        let path = dir.join("greys.mp4");
        let mut child = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
            .args(["-pix_fmt", "rgb24", "-s", "16x16", "-r", "7", "-i", "-"])
            .args(["-c:v", "libx264rgb", "-qp", "0"])
            .arg(&path)
            .stdin(Stdio::piped())
            .spawn()
            .ok()?;
        let stdin = child.stdin.as_mut()?;
        for k in 0..30u8 {
            stdin.write_all(&[8 + 8 * k; 3].repeat(16 * 16)).ok()?;
        }
        drop(child.stdin.take());
        child.wait().ok()?.success().then_some(path)
    }

    #[test]
    fn a_decoder_started_between_the_files_frames_gives_the_one_on_screen_then() {
        let dir = std::env::temp_dir().join(format!("zgx-clip-seek-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let Some(src) = greys(&dir) else {
            eprintln!("no ffmpeg with libx264rgb here: skipped");
            return;
        };
        // Each a decoder of its own, asked first for frame n of a 30 fps
        // video: n / 30 s in, past a second so it seeks a second early, and
        // between two of the file's frames. On screen then: the last of its
        // frames to start at or before it, 7n / 30 rounded down.
        for n in [34u64, 40, 47, 61, 64, 90, 121] {
            let clip = Clip {
                id: "greys".into(),
                src: src.clone(),
                from: 0.,
                duration: None,
                trim: 0.,
                size: (16, 16),
                looped: false,
            };
            let mut decoder = Decoder::new(clip, 30.);
            let pixels = decoder.frame(n).unwrap();
            let shown = (i32::from(pixels[0]) - 8 + 4).div_euclid(8);
            assert_eq!(shown, (7 * n / 30) as i32, "started at frame {n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
