//! Where frames go: an encoder fed raw frames (ffmpeg, found on the PATH),
//! PNG files, or a GIF. A sink runs on a thread of its own, so a frame being
//! encoded doesn't hold up the next being drawn.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

/// What a video is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// H.264 in an MP4.
    H264,
    /// HEVC in an MP4.
    Hevc,
    /// ProRes 4444 in a MOV, alpha kept.
    ProRes,
    /// VP9 in a WebM, alpha kept, Opus sound: for the web.
    WebM,
    /// AV1 in an MP4: smaller than H.264 at the same quality.
    Av1,
    /// A numbered PNG for each frame, alpha kept.
    Png,
    /// An animated GIF.
    Gif,
}
impl Format {
    /// The format a name says (`h264`, `hevc`, `prores`, `vp9` or `webm`,
    /// `av1`, `png`, `gif`).
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "h264" | "avc" | "mp4" => Self::H264,
            "hevc" | "h265" => Self::Hevc,
            "prores" | "mov" => Self::ProRes,
            "vp9" | "webm" => Self::WebM,
            "av1" => Self::Av1,
            "png" => Self::Png,
            "gif" => Self::Gif,
            _ => return None,
        })
    }
    /// The format an output path's extension says.
    pub fn of_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "mp4" | "m4v" => Some(Self::H264),
            "mov" => Some(Self::ProRes),
            "webm" => Some(Self::WebM),
            "gif" => Some(Self::Gif),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
    /// Whether it keeps alpha.
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::ProRes | Self::WebM | Self::Png | Self::Gif)
    }
    /// Whether ffmpeg encodes it, with its sound muxed in.
    pub fn is_encoded(self) -> bool {
        matches!(
            self,
            Self::H264 | Self::Hevc | Self::ProRes | Self::WebM | Self::Av1
        )
    }
}

/// How to write a video.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// Its format.
    pub format: Format,
    /// Where: a file, or for PNGs a directory, or a path holding `%04d`.
    pub output: PathBuf,
    /// Its size in pixels.
    pub size: (u32, u32),
    /// Frames per second.
    pub fps: f64,
    /// Constant quality, lower being better (`libx264`'s scale, 16 by default;
    /// VideoToolbox's is mapped to it).
    pub quality: Option<u32>,
    /// A WAV to mux in.
    pub audio: Option<PathBuf>,
    /// The number its first frame takes in a PNG sequence (a chunk of a
    /// larger render's).
    pub first_index: usize,
}

/// ffmpeg: on the PATH, or where a package manager puts it for an app the
/// system started, whose PATH is short.
pub fn ffmpeg() -> Option<PathBuf> {
    let name = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(
            ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
                .iter()
                .map(PathBuf::from),
        )
        .map(|dir| dir.join(&name))
        .find(|file| file.is_file())
}

const NO_FFMPEG: &str = "zgx video needs ffmpeg for this format, and found none on the PATH. Install it (`brew install ffmpeg`, `apt install ffmpeg`, `winget install ffmpeg`), or render PNGs or a GIF, which need nothing";

/// A frame, its pixels straight or premultiplied as the sink was told.
pub type Pixels = Vec<u8>;

/// Takes frames on a thread of its own.
pub struct Sink {
    frames: Option<SyncSender<Pixels>>,
    spare: Receiver<Pixels>,
    worker: Option<JoinHandle<Result<(), String>>>,
}
impl Sink {
    /// Start writing.
    pub fn start(options: EncodeOptions) -> Result<Self, String> {
        let mut encoder = Encoder::start(&options)?;
        let (frames, incoming) = sync_channel::<Pixels>(4);
        let (give_back, spare) = sync_channel::<Pixels>(8);
        let worker = std::thread::Builder::new()
            .name("zgx-video encode".into())
            .spawn(move || {
                for (index, frame) in incoming.into_iter().enumerate() {
                    encoder.write(&frame, index)?;
                    let _ = give_back.try_send(frame);
                }
                encoder.finish()
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            frames: Some(frames),
            spare,
            worker: Some(worker),
        })
    }
    /// A buffer to read the next frame into: one the encoder has finished with, if any.
    pub fn buffer(&self) -> Pixels {
        self.spare.try_recv().unwrap_or_default()
    }
    /// Queue a frame, waiting while the encoder is behind.
    pub fn push(&mut self, frame: Pixels) -> Result<(), String> {
        let sent = self
            .frames
            .as_ref()
            .is_some_and(|frames| frames.send(frame).is_ok());
        if sent {
            return Ok(());
        }
        // The worker stopped: what it stopped for says more.
        Err(self
            .join()
            .err()
            .unwrap_or_else(|| "the encoder stopped".into()))
    }
    /// All frames sent: wait for the file.
    pub fn finish(mut self) -> Result<(), String> {
        self.join()
    }
    fn join(&mut self) -> Result<(), String> {
        self.frames = None;
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| "the encoder panicked".to_owned())?,
            None => Ok(()),
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

enum Encoder {
    Ffmpeg {
        child: Child,
        stderr: JoinHandle<String>,
        output: PathBuf,
    },
    Pngs {
        pattern: PathBuf,
        size: (u32, u32),
        first: usize,
    },
    Gif {
        writer: Box<gif::Encoder<std::io::BufWriter<std::fs::File>>>,
        size: (u16, u16),
        fps: f64,
    },
}

/// Numbered file `index` of a PNG `pattern`.
fn numbered(pattern: &Path, index: usize) -> PathBuf {
    let text = pattern.to_string_lossy();
    match text.find("%04d") {
        Some(at) => PathBuf::from(format!("{}{index:04}{}", &text[..at], &text[at + 4..])),
        None => pattern.join(format!("{index:05}.png")),
    }
}

impl Encoder {
    fn start(options: &EncodeOptions) -> Result<Self, String> {
        if let Some(parent) = options.output.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        match options.format {
            Format::Png => {
                let has_pattern = options.output.to_string_lossy().contains("%04d");
                if !has_pattern {
                    std::fs::create_dir_all(&options.output)
                        .map_err(|error| format!("{}: {error}", options.output.display()))?;
                }
                Ok(Self::Pngs {
                    pattern: options.output.clone(),
                    size: options.size,
                    first: options.first_index,
                })
            }
            Format::Gif => {
                let (width, height) = options.size;
                let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
                    return Err("a GIF is at most 65535 pixels a side".into());
                };
                let file = std::fs::File::create(&options.output)
                    .map_err(|error| format!("{}: {error}", options.output.display()))?;
                let mut writer =
                    gif::Encoder::new(std::io::BufWriter::new(file), width, height, &[])
                        .map_err(|error| error.to_string())?;
                writer
                    .set_repeat(gif::Repeat::Infinite)
                    .map_err(|error| error.to_string())?;
                Ok(Self::Gif {
                    writer: Box::new(writer),
                    size: (width, height),
                    fps: options.fps,
                })
            }
            Format::H264 | Format::Hevc | Format::ProRes | Format::WebM | Format::Av1 => {
                Self::ffmpeg(options)
            }
        }
    }
    fn ffmpeg(options: &EncodeOptions) -> Result<Self, String> {
        let ffmpeg = ffmpeg().ok_or(NO_FFMPEG)?;
        let encoders = Command::new(&ffmpeg)
            .args(["-hide_banner", "-encoders"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        let has = |name: &str| encoders.split_whitespace().any(|word| word == name);
        let mut command = Command::new(&ffmpeg);
        let (width, height) = options.size;
        // sRGB-coded pixels, as the renderer writes them.
        command.args(["-hide_banner", "-loglevel", "error", "-y"]);
        command.args(["-f", "rawvideo", "-pix_fmt", "rgba"]);
        command.args(["-s", &format!("{width}x{height}")]);
        command.args(["-r", &options.fps.to_string()]);
        command.args(["-i", "-"]);
        if let Some(audio) = &options.audio {
            command.arg("-i").arg(audio);
        }
        let quality = options.quality.unwrap_or(16);
        // Rec. 709 for what's limited range, as players assume of HD video and
        // tagged so none has to guess. 4:2:0 needs even sides.
        let limited = "scale=trunc(iw/2)*2:trunc(ih/2)*2:flags=lanczos+accurate_rnd+full_chroma_int:in_color_matrix=bt709:out_color_matrix=bt709:in_range=pc:out_range=tv,format=yuv420p,setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709";
        match options.format {
            Format::H264 if has("libx264") => {
                command.args(["-vf", limited, "-c:v", "libx264", "-preset", "medium"]);
                command.args(["-crf", &quality.to_string()]);
            }
            Format::H264 if has("h264_videotoolbox") => {
                command.args(["-vf", limited, "-c:v", "h264_videotoolbox"]);
                command.args([
                    "-q:v",
                    &(100 - quality.min(50) * 2 + 10).min(100).to_string(),
                ]);
            }
            Format::H264 => {
                return Err(
                    "this ffmpeg has no H.264 encoder (libx264 or h264_videotoolbox)".into(),
                );
            }
            Format::Hevc if has("libx265") => {
                command.args(["-vf", limited, "-c:v", "libx265", "-preset", "medium"]);
                command.args(["-crf", &(quality + 4).to_string(), "-tag:v", "hvc1"]);
                command.args(["-x265-params", "log-level=error"]);
            }
            Format::Hevc if has("hevc_videotoolbox") => {
                command.args([
                    "-vf",
                    limited,
                    "-c:v",
                    "hevc_videotoolbox",
                    "-tag:v",
                    "hvc1",
                ]);
                command.args([
                    "-q:v",
                    &(100 - quality.min(50) * 2 + 5).min(100).to_string(),
                ]);
            }
            Format::Hevc => {
                return Err(
                    "this ffmpeg has no HEVC encoder (libx265 or hevc_videotoolbox)".into(),
                );
            }
            Format::ProRes => {
                command.args(["-vf", "scale=in_color_matrix=bt709:out_color_matrix=bt709:in_range=pc:out_range=tv,format=yuva444p10le,setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709"]);
                command.args(["-c:v", "prores_ks", "-profile:v", "4444"]);
                command.args(["-pix_fmt", "yuva444p10le", "-alpha_bits", "16"]);
                command.args(["-vendor", "apl0"]);
            }
            Format::WebM if has("libvpx-vp9") => {
                // Alpha as VP9 keeps it: 4:2:0 with an alpha plane, which
                // browsers play; no alternate reference frames, which drop it.
                command.args(["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2:flags=lanczos+accurate_rnd+full_chroma_int:in_color_matrix=bt709:out_color_matrix=bt709:in_range=pc:out_range=tv,format=yuva420p,setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709"]);
                command.args(["-c:v", "libvpx-vp9", "-b:v", "0"]);
                command.args(["-crf", &(quality + 15).min(63).to_string()]);
                command.args(["-row-mt", "1", "-deadline", "good", "-cpu-used", "2"]);
                command.args(["-auto-alt-ref", "0"]);
            }
            Format::WebM => return Err("this ffmpeg has no VP9 encoder (libvpx-vp9)".into()),
            Format::Av1 if has("libsvtav1") => {
                command.args(["-vf", limited, "-c:v", "libsvtav1", "-preset", "6"]);
                command.args(["-crf", &(quality + 14).min(63).to_string()]);
                command.args(["-svtav1-params", "tune=0"]);
            }
            Format::Av1 if has("libaom-av1") => {
                command.args(["-vf", limited, "-c:v", "libaom-av1", "-cpu-used", "6"]);
                command.args(["-crf", &(quality + 14).min(63).to_string(), "-b:v", "0"]);
            }
            Format::Av1 => {
                return Err("this ffmpeg has no AV1 encoder (libsvtav1 or libaom-av1)".into());
            }
            _ => unreachable!("not ffmpeg's"),
        }
        command.args(["-colorspace", "bt709", "-color_primaries", "bt709"]);
        command.args(["-color_trc", "bt709", "-color_range", "tv"]);
        if options.audio.is_some() {
            match options.format {
                Format::ProRes => command.args(["-c:a", "pcm_s24le"]),
                Format::WebM => command.args(["-c:a", "libopus", "-b:a", "192k"]),
                _ => command.args(["-c:a", "aac", "-b:a", "256k"]),
            };
            // No `-shortest`: the mix is never longer than the frames (see
            // `render::mix_range`), and to end at the shorter stream ffmpeg
            // holds 10 s of decoded frames, 1.8 GB at 1080p and 60 fps.
        }
        if matches!(options.format, Format::H264 | Format::Hevc | Format::Av1) {
            command.args(["-movflags", "+faststart"]);
        }
        command.arg(&options.output);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("couldn't run {}: {error}", ffmpeg.display()))?;
        let mut err = child.stderr.take().expect("piped");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut err, &mut text);
            text
        });
        Ok(Self::Ffmpeg {
            child,
            stderr,
            output: options.output.clone(),
        })
    }
    fn write(&mut self, frame: &[u8], index: usize) -> Result<(), String> {
        match self {
            Self::Ffmpeg { child, .. } => child
                .stdin
                .as_mut()
                .expect("piped")
                .write_all(frame)
                .map_err(|error| format!("ffmpeg stopped taking frames: {error}")),
            Self::Pngs {
                pattern,
                size,
                first,
            } => {
                let path = numbered(pattern, *first + index);
                let file = std::fs::File::create(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size.0, size.1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
                encoder.set_compression(png::Compression::Fast);
                let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
                writer
                    .write_image_data(frame)
                    .map_err(|error| error.to_string())
            }
            Self::Gif { writer, size, fps } => {
                let mut rgba = frame.to_vec();
                let mut out = gif::Frame::from_rgba_speed(size.0, size.1, &mut rgba, 10);
                // In hundredths of a second, never drifting from the frame rate.
                let at = |n: usize| (n as f64 * 100. / *fps).round() as i64;
                out.delay = (at(index + 1) - at(index)).max(1) as u16;
                writer.write_frame(&out).map_err(|error| error.to_string())
            }
        }
    }
    fn finish(self) -> Result<(), String> {
        match self {
            Self::Ffmpeg {
                mut child,
                stderr,
                output,
            } => {
                drop(child.stdin.take());
                let status = child.wait().map_err(|error| error.to_string())?;
                let said = stderr.join().unwrap_or_default();
                if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "ffmpeg couldn't write {}: {}",
                        output.display(),
                        said.trim()
                    ))
                }
            }
            Self::Pngs { .. } | Self::Gif { .. } => Ok(()),
        }
    }
}

/// Premultiplied pixels made straight, in place.
pub fn straighten(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

/// Join `parts` (segments of one encoding, in order) into `output` without
/// encoding them again, muxing `audio` in if given.
pub fn concat(
    parts: &[PathBuf],
    output: &Path,
    format: Format,
    audio: Option<&Path>,
) -> Result<(), String> {
    let ffmpeg = ffmpeg().ok_or(NO_FFMPEG)?;
    let list = output.with_file_name(format!(
        ".{}.parts.txt",
        output
            .file_name()
            .map_or("out".into(), |name| name.to_string_lossy())
    ));
    let text: String = parts
        .iter()
        .map(|part| {
            let path = std::path::absolute(part).unwrap_or_else(|_| part.clone());
            format!("file '{}'\n", path.to_string_lossy().replace('\'', "'\\''"))
        })
        .collect();
    std::fs::write(&list, text).map_err(|error| error.to_string())?;
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    // One part is read as it is: through the concat demuxer, its keyframes
    // would carry the parameter sets again, and the frames it copies would
    // differ from those encoded.
    if let [part] = parts {
        command.arg("-i").arg(part);
    } else {
        command
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list);
    }
    if let Some(audio) = audio {
        command.arg("-i").arg(audio);
        command.args(["-map", "0:v", "-map", "1:a"]);
    }
    command.args(["-c:v", "copy"]);
    if audio.is_some() {
        match format {
            Format::ProRes => command.args(["-c:a", "pcm_s24le"]),
            Format::WebM => command.args(["-c:a", "libopus", "-b:a", "192k"]),
            _ => command.args(["-c:a", "aac", "-b:a", "256k"]),
        };
        // No `-shortest`: the mix is never longer than the frames, and a
        // shorter one would have it end the file early, cutting frames.
    }
    if matches!(format, Format::H264 | Format::Hevc | Format::Av1) {
        command.args(["-movflags", "+faststart"]);
    }
    let out = command
        .arg(output)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("couldn't run ffmpeg: {error}"));
    let _ = std::fs::remove_file(&list);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg couldn't join {}: {}",
            output.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}
