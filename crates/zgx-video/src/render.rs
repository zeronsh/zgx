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
    /// Chunks of frames rendered at once, each on a thread of its own with a
    /// stage and an encoder of its own (1: one after another). A GIF is
    /// always made in one.
    pub concurrency: usize,
    /// Keep one frame in this many (1: all), the file's rate divided by it:
    /// a GIF made light.
    pub every_nth: u64,
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
            concurrency: 1,
            every_nth: 1,
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
                    "{}: say which format with --codec (h264, hevc, prores, vp9, av1, png, gif)",
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
    if options.concurrency > 1 && options.resolved_format()? != Format::Gif {
        return render_chunks(options, progress);
    }
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
    let takes_sound = format.is_encoded();
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
        fps: ticks.output_fps / options.every_nth.max(1) as f64,
        quality: options.quality,
        audio: audio.clone(),
        first_index: 0,
    })?;
    let every = options.every_nth.max(1);
    let total = (end - first) / every + 1;
    let film = playing.film();
    let result = (|| {
        for (done, frame) in (first..=end).step_by(every as usize).enumerate() {
            let mut pixels = sink.buffer();
            film.draw_output(frame, &mut pixels, options.hold_timeout)?;
            if transparent {
                straighten(&mut pixels);
            }
            sink.push(pixels)?;
            if !progress(Progress {
                done: done as u64 + 1,
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

/// The mix of `clips` over frames `first` to `end` of a file at `output_fps`:
/// in whole frames of the composition, rounded down, so it's never longer
/// than the frames and the encoder needn't cut it (or end the video early,
/// a mix rounded short having cut its last frame).
fn mix_range(
    clips: Vec<AudioClip>,
    fps: f64,
    output_fps: f64,
    first: u64,
    end: u64,
) -> Result<Option<PathBuf>, String> {
    // A whole count computed a hair under stays whole.
    let frames = ((end + 1 - first) as f64 / output_fps * fps + 1e-9).floor() as u64;
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

/// [`render`], in `options.concurrency` chunks of frames at once: each on a
/// thread of its own, with a virtual clock, a stage and an encoder of its own.
/// A chunk moves to its first frame without drawing the ones before it, and
/// draws what a render of them all draws there. PNGs are numbered as one
/// sequence; an encoded video's chunks are joined, not encoded again.
fn render_chunks(
    options: &RenderOptions,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, String> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let started = Instant::now();
    let format = options.resolved_format()?;
    let transparent = options.transparent && format.has_alpha();
    let background = if transparent {
        Color(0, 0, 0, 0)
    } else {
        Color(0, 0, 0, 255)
    };
    // The composition's measure, as its props make it.
    let list = crate::script::compositions(
        &options.script,
        options.composition.as_deref(),
        &options.props,
    )?;
    let composition = match &options.composition {
        Some(id) => list
            .into_iter()
            .find(|comp| &comp.id == id)
            .ok_or_else(|| format!("no composition `{id}`"))?,
        None => list
            .into_iter()
            .next()
            .expect("compositions() returns some"),
    };
    let ticks = crate::film::Ticks::of(options.timing(), composition.fps);
    let frames = ((composition.duration as f64 / composition.fps) * ticks.output_fps)
        .round()
        .max(1.) as u64;
    let last = frames - 1;
    let (first, end) = options.frames.unwrap_or((0, last));
    let end = end.min(last);
    if first > end {
        return Err(format!(
            "frames {first} to {end} hold nothing; it has {frames} frames"
        ));
    }
    let size = (
        ((composition.width * options.scale).round() as u32).max(1),
        ((composition.height * options.scale).round() as u32).max(1),
    );
    // The frames kept, one in `every_nth`, split by their place among them.
    let every = options.every_nth.max(1);
    let total = (end - first) / every + 1;
    let count = (options.concurrency as u64).clamp(1, total);
    let chunks: Vec<(u64, u64)> = (0..count)
        .map(|k| (total * k / count, total * (k + 1) / count))
        .filter(|(from, to)| from < to)
        .collect();
    let encoded = format.is_encoded();
    let audio = if options.mute || !encoded {
        None
    } else {
        let (clips, fps) = placed_audio(options)?;
        mix_range(clips, fps, ticks.output_fps, first, end)?
    };
    let part = |k: usize| -> PathBuf {
        let name = options
            .output
            .file_name()
            .map_or("out".into(), |name| name.to_string_lossy().into_owned());
        let ext = options
            .output
            .extension()
            .map_or("mp4".into(), |ext| ext.to_string_lossy().into_owned());
        options
            .output
            .with_file_name(format!(".{name}.part{k}.{ext}"))
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (tell, heard) = std::sync::mpsc::channel::<Result<(), String>>();
    let outcome = std::thread::scope(|scope| {
        for (k, &(from, to)) in chunks.iter().enumerate() {
            let (tell, stop) = (tell.clone(), stop.clone());
            let output = if encoded {
                part(k)
            } else {
                options.output.clone()
            };
            let options = options.clone();
            scope.spawn(move || {
                let result = (|| {
                    let mut stage = ScriptStage::new(
                        &options.script,
                        options.composition.as_deref(),
                        &options.props,
                        options.scale,
                        background,
                        options.timing(),
                    )?;
                    let mut sink = Sink::start(EncodeOptions {
                        format,
                        output,
                        size,
                        fps: ticks.output_fps / every as f64,
                        quality: options.quality,
                        audio: None,
                        first_index: from as usize,
                    })?;
                    for frame in (from..to).map(|place| first + place * every) {
                        if stop.load(Ordering::Relaxed) {
                            return Err("stopped".to_owned());
                        }
                        let mut pixels = sink.buffer();
                        stage.draw_output(frame, &mut pixels, options.hold_timeout)?;
                        if transparent {
                            straighten(&mut pixels);
                        }
                        sink.push(pixels)?;
                        let _ = tell.send(Ok(()));
                    }
                    drop(stage);
                    sink.finish()
                })();
                if let Err(error) = result {
                    stop.store(true, Ordering::Relaxed);
                    let _ = tell.send(Err(error));
                }
            });
        }
        drop(tell);
        let mut done = 0;
        let mut failed = None;
        for message in heard {
            match message {
                Ok(()) => {
                    done += 1;
                    if !progress(Progress { done, total }) {
                        stop.store(true, Ordering::Relaxed);
                        failed.get_or_insert_with(|| "stopped".to_owned());
                    }
                }
                Err(error) => {
                    // The first chunk's error says why; the others only stopped.
                    if failed.is_none() || failed.as_deref() == Some("stopped") {
                        failed = Some(error);
                    }
                }
            }
        }
        failed.map_or(Ok(()), Err)
    });
    let joined = outcome.and_then(|()| {
        if encoded {
            let parts: Vec<PathBuf> = (0..chunks.len()).map(part).collect();
            crate::encode::concat(&parts, &options.output, format, audio.as_deref())
        } else {
            Ok(())
        }
    });
    if encoded {
        for k in 0..chunks.len() {
            let _ = std::fs::remove_file(part(k));
        }
    }
    if let Some(audio) = audio {
        let _ = std::fs::remove_file(audio);
    }
    joined?;
    let elapsed = started.elapsed();
    Ok(Report {
        output: options.output.clone(),
        frames: total,
        size,
        elapsed,
        rate: total as f64 / elapsed.as_secs_f64().max(1e-6),
    })
}

/// One frame of a composition, its pixels straight RGBA.
#[derive(Clone, Debug)]
pub struct Still {
    /// Its width in pixels.
    pub width: u32,
    /// Its height.
    pub height: u32,
    /// Its pixels, a row at a time.
    pub pixels: Vec<u8>,
}
impl Still {
    /// Written as a PNG at `path`.
    pub fn write_png(&self, path: &Path) -> Result<(), String> {
        let file =
            std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&self.pixels)
            .map_err(|error| error.to_string())
    }
}

/// Frame `frame` of the composition `options` names, drawn as a render of the
/// whole would draw it, without encoding anything: a still, or with
/// `max_width` a thumbnail no wider than that many pixels. `options.output`
/// and `options.format` are unused; `transparent` keeps its background clear.
pub fn render_still(
    options: &RenderOptions,
    frame: u64,
    max_width: Option<u32>,
) -> Result<Still, String> {
    let mut scale = options.scale;
    if let Some(widest) = max_width {
        let list = crate::script::compositions(
            &options.script,
            options.composition.as_deref(),
            &options.props,
        )?;
        let width = match &options.composition {
            Some(id) => list
                .iter()
                .find(|comp| &comp.id == id)
                .map(|comp| comp.width),
            None => list.first().map(|comp| comp.width),
        }
        .ok_or("no such composition")?;
        scale = scale.min(widest as f32 / width.max(1.));
    }
    let background = if options.transparent {
        Color(0, 0, 0, 0)
    } else {
        Color(0, 0, 0, 255)
    };
    let mut stage = ScriptStage::new(
        &options.script,
        options.composition.as_deref(),
        &options.props,
        scale,
        background,
        options.timing(),
    )?;
    let last = stage.output_frames().saturating_sub(1);
    if frame > last {
        return Err(format!("no frame {frame}; it has {} frames", last + 1));
    }
    let mut pixels = Vec::new();
    stage.draw_output(frame, &mut pixels, options.hold_timeout)?;
    straighten(&mut pixels);
    let (width, height) = stage.stage.pixel_size();
    Ok(Still {
        width,
        height,
        pixels,
    })
}
