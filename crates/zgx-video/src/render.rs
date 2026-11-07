//! A composition rendered to a file.
use crate::encode::{EncodeOptions, Format, Sink, straighten};
use crate::film::{Film, Timing};
use crate::script::{AudioClip, ScriptStage};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zgx::scene::Color;

/// How to render a composition.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Its script, or a directory holding it.
    pub script: PathBuf,
    /// Which composition; the first if `None`.
    pub composition: Option<String>,
    /// Where it goes.
    pub output: PathBuf,
    /// As what; by `output`'s extension if `None`.
    pub format: Option<Format>,
    /// What its props take over their defaults.
    pub props: Json,
    /// Pixels per point.
    pub scale: f32,
    /// Frames per second of the file, if not the composition's: it plays at
    /// the same speed, with more frames.
    pub fps: Option<f64>,
    /// First and last frame kept (of the file, inclusive).
    pub frames: Option<(u64, u64)>,
    /// A transparent background, for formats keeping alpha.
    pub transparent: bool,
    /// Constant quality (see [`EncodeOptions::quality`]).
    pub quality: Option<u32>,
    /// Frames blended into each frame, for motion blur (1: none).
    pub samples: u32,
    /// How much of a frame's time its samples span, from 0 to 1.
    pub shutter: f64,
    /// Leave its sounds out.
    pub mute: bool,
    /// How long a frame may wait for what the script holds it for.
    pub hold_timeout: Duration,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            script: PathBuf::new(),
            composition: None,
            output: PathBuf::new(),
            format: None,
            props: Json::Object(Default::default()),
            scale: 1.,
            fps: None,
            frames: None,
            transparent: false,
            quality: None,
            samples: 1,
            shutter: 0.5,
            mute: false,
            hold_timeout: Duration::from_secs(60),
        }
    }
}

/// How far a render has come.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Frames written.
    pub done: u64,
    /// Frames it writes.
    pub total: u64,
}

/// What a render did.
#[derive(Clone, Debug)]
pub struct Report {
    /// What it wrote.
    pub output: PathBuf,
    /// Frames written.
    pub frames: u64,
    /// Their size, in pixels.
    pub size: (u32, u32),
    /// How long it took.
    pub elapsed: Duration,
    /// Frames per second it made them at.
    pub rate: f64,
}

impl RenderOptions {
    /// How its frames become a file's.
    pub fn timing(&self) -> Timing {
        Timing {
            fps: self.fps,
            samples: self.samples,
            shutter: self.shutter,
        }
    }
    fn resolved_format(&self) -> Result<Format, String> {
        match self.format {
            Some(format) => Ok(format),
            None if is_sequence(&self.output) => Ok(Format::Png),
            None => Format::of_path(&self.output).ok_or_else(|| {
                format!(
                    "{}: say which format with --codec (h264, hevc, prores, png, gif)",
                    self.output.display()
                )
            }),
        }
    }
}

/// Render a script's composition; `progress` hears after each frame, and stops
/// it by returning false.
pub fn render(
    options: &RenderOptions,
    progress: impl FnMut(Progress) -> bool,
) -> Result<Report, String> {
    let timing = options.timing();
    render_film(options, progress, |background| {
        ScriptStage::new(
            &options.script,
            options.composition.as_deref(),
            &options.props,
            options.scale,
            background,
            timing,
        )
    })
}

/// Something that plays and may place sounds.
pub(crate) trait Playing {
    fn film(&mut self) -> &mut Film;
    fn sounds(&mut self, options: &RenderOptions) -> Result<Option<(Vec<AudioClip>, f64)>, String>;
}
impl Playing for Film {
    fn film(&mut self) -> &mut Film {
        self
    }
    fn sounds(&mut self, _: &RenderOptions) -> Result<Option<(Vec<AudioClip>, f64)>, String> {
        Ok(None)
    }
}
impl Playing for ScriptStage {
    fn film(&mut self) -> &mut Film {
        &mut self.film
    }
    fn sounds(&mut self, options: &RenderOptions) -> Result<Option<(Vec<AudioClip>, f64)>, String> {
        placed_audio(options).map(Some)
    }
}

/// Render what `make` stages for the background it's given.
pub(crate) fn render_film<P: Playing>(
    options: &RenderOptions,
    mut progress: impl FnMut(Progress) -> bool,
    make: impl FnOnce(Color) -> Result<P, String>,
) -> Result<Report, String> {
    let started = Instant::now();
    let format = options.resolved_format()?;
    let transparent = options.transparent && format.has_alpha();
    let background = if transparent {
        Color(0, 0, 0, 0)
    } else {
        Color(0, 0, 0, 255)
    };
    let mut playing = make(background)?;
    let film = playing.film();
    let ticks = film.ticks();
    let last = film.output_frames().saturating_sub(1);
    let (first, end) = options.frames.unwrap_or((0, last));
    let end = end.min(last);
    if first > end {
        return Err(format!(
            "frames {first} to {end} hold nothing; it has {} frames",
            last + 1
        ));
    }
    let size = film.stage.pixel_size();
    let takes_sound = matches!(format, Format::H264 | Format::Hevc | Format::ProRes);
    let audio = if options.mute || !takes_sound {
        None
    } else {
        match playing.sounds(options)? {
            Some((clips, fps)) => mix_range(clips, fps, ticks.output_fps, first, end)?,
            None => None,
        }
    };
    let mut sink = Sink::start(EncodeOptions {
        format,
        output: options.output.clone(),
        size,
        fps: ticks.output_fps,
        quality: options.quality,
        audio: audio.clone(),
    })?;
    let total = end - first + 1;
    let film = playing.film();
    let result = (|| {
        for frame in first..=end {
            let mut pixels = sink.buffer();
            film.draw_output(frame, &mut pixels, options.hold_timeout)?;
            if transparent {
                straighten(&mut pixels);
            }
            sink.push(pixels)?;
            if !progress(Progress {
                done: frame - first + 1,
                total,
            }) {
                return Err("stopped".to_owned());
            }
        }
        Ok(())
    })();
    drop(playing);
    let finished = sink.finish();
    if let Some(audio) = audio {
        let _ = std::fs::remove_file(audio);
    }
    result?;
    finished?;
    let elapsed = started.elapsed();
    Ok(Report {
        output: options.output.clone(),
        frames: total,
        size,
        elapsed,
        rate: total as f64 / elapsed.as_secs_f64().max(1e-6),
    })
}

/// What the script places, played through on a thread of its own, drawing
/// nothing: a sound may be placed by a sequence only its frame mounts. Its
/// clips, and the composition's rate they count in.
fn placed_audio(options: &RenderOptions) -> Result<(Vec<AudioClip>, f64), String> {
    let (script, composition, props) = (
        options.script.clone(),
        options.composition.clone(),
        options.props.clone(),
    );
    std::thread::scope(|scope| {
        scope
            .spawn(move || -> Result<(Vec<AudioClip>, f64), String> {
                let mut stage = ScriptStage::new(
                    &script,
                    composition.as_deref(),
                    &props,
                    0.1,
                    Color(0, 0, 0, 255),
                    Timing::default(),
                )?;
                let fps = stage.composition().fps;
                let frames = stage.composition().duration;
                stage.film.step_to(frames.saturating_sub(1))?;
                Ok((stage.audio(), fps))
            })
            .join()
            .map_err(|_| "the audio pass panicked".to_owned())?
    })
}

/// The mix of `clips` over frames `first` to `end` of a file at `output_fps`.
fn mix_range(
    clips: Vec<AudioClip>,
    fps: f64,
    output_fps: f64,
    first: u64,
    end: u64,
) -> Result<Option<PathBuf>, String> {
    let frames = ((end + 1 - first) as f64 / output_fps * fps).round() as u64;
    let skip = (first as f64 / output_fps * fps).round() as i64;
    let clips: Vec<_> = clips
        .into_iter()
        .map(|mut clip| {
            let start = clip.from as i64 - skip;
            if start < 0 {
                clip.trim += (-start) as u64;
                clip.duration = clip
                    .duration
                    .map(|lasts| lasts.saturating_sub((-start) as u64));
                clip.from = 0;
            } else {
                clip.from = start as u64;
            }
            clip
        })
        .collect();
    crate::audio::mixed_file(&clips, fps, frames)
}

/// Whether `path` names a frame pattern or directory rather than a file.
pub fn is_sequence(path: &Path) -> bool {
    path.extension().is_none() || path.to_string_lossy().contains("%04d")
}
