//! Compositions written in Luau: `require("@video")` (`luau/video.luau`),
//! read and staged here.
use crate::film::{Film, Player, Ticks, Timing};
use crate::stage::{Setting, Stage, StageOptions};
use serde_json::{Value as Json, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use zgx_script::{ScriptHost, Shared, Value};

/// A composition a script declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Composition {
    /// Its name.
    pub id: String,
    /// Width, in points.
    pub width: f32,
    /// Height, in points.
    pub height: f32,
    /// Frames per second.
    pub fps: f64,
    /// How many frames it has.
    pub duration: u64,
    /// What it's given: its defaults, with the input over them.
    pub props: Json,
    /// What it takes when it's given nothing.
    pub defaults: Json,
    /// What each prop may be, for the studio's panel: `{ name: { min, max, step, options } }`.
    pub schema: Json,
}
impl Composition {
    fn from_json(value: &Json) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            width: value.get("w")?.as_f64()? as f32,
            height: value.get("h")?.as_f64()? as f32,
            fps: value.get("fps")?.as_f64()?,
            duration: value.get("duration")?.as_f64()?.max(1.) as u64,
            props: value.get("props").cloned().unwrap_or(json!({})),
            defaults: value.get("defaults").cloned().unwrap_or(json!({})),
            schema: value.get("schema").cloned().unwrap_or(json!({})),
        })
    }
}

/// A sequence a script has shown, for a timeline.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Track {
    /// Its name.
    pub name: String,
    /// Its first frame.
    pub from: u64,
    /// How many frames it lasts; `None`, on to the end.
    pub duration: Option<u64>,
}

/// A sound a script has placed.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    /// Its file.
    pub src: PathBuf,
    /// The frame it starts at.
    pub from: u64,
    /// How many frames it lasts; `None`, as long as it runs.
    pub duration: Option<u64>,
    /// Frames of it skipped.
    pub trim: u64,
    /// Its loudness, 1 as it is.
    pub volume: f64,
    /// Frames it fades in over.
    pub fade_in: u64,
    /// Frames it fades out over, at its end.
    pub fade_out: u64,
    /// It repeats until its duration.
    pub looped: bool,
    /// A video's sound: silence if it has none.
    pub optional: bool,
    /// Its volume at each frame from its start (`volume` given as a function),
    /// gone between; `volume` scales it.
    pub volumes: Option<Vec<f32>>,
}

/// The source of the Luau side, for `require("@video")`.
const VIDEO_LUAU: &str = include_str!("../luau/video.luau");
/// zgx-fx's effects, for `require("@fx")`.
const FX_LUAU: &str = include_str!("../../zgx-fx/luau/fx.luau");

/// The packages scripts require, written where their host can read them,
/// once a process.
fn packages() -> Result<(PathBuf, PathBuf), String> {
    static WRITTEN: std::sync::OnceLock<Result<(PathBuf, PathBuf), String>> =
        std::sync::OnceLock::new();
    WRITTEN
        .get_or_init(|| write_packages().map_err(|error| error.to_string()))
        .clone()
}
fn write_packages() -> std::io::Result<(PathBuf, PathBuf)> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    VIDEO_LUAU.hash(&mut hasher);
    FX_LUAU.hash(&mut hasher);
    let root = std::env::temp_dir().join(format!("zgx-video-{:016x}", hasher.finish()));
    let (video, fx) = (root.join("video"), root.join("fx"));
    for (dir, code) in [(&video, VIDEO_LUAU), (&fx, FX_LUAU)] {
        let file = dir.join("init.luau");
        if std::fs::read_to_string(&file).is_ok_and(|on_disk| on_disk == code) {
            continue;
        }
        std::fs::create_dir_all(dir)?;
        // Whole or not at all: another process may be reading it.
        let partial = dir.join(format!("init.{}.tmp", std::process::id()));
        std::fs::write(&partial, code)?;
        std::fs::rename(&partial, &file)?;
    }
    Ok((video, fx))
}

/// The fonts in the `fonts` folder beside a composition's script (`.ttf`,
/// `.otf`, `.ttc`), for its text to name by family. One that isn't a font is
/// said on stderr and left out.
pub fn fonts_beside(entry: &Path) -> Vec<zgx_gpu::text::FontData> {
    let Some(dir) = entry.parent().map(|dir| dir.join("fonts")) else {
        return Vec::new();
    };
    let Ok(files) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = files
        .filter_map(|file| file.ok().map(|file| file.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc")
                })
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let font = std::fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| zgx_gpu::text::FontData::new(bytes).map_err(str::to_owned));
            font.map_err(|error| eprintln!("zgx-video: {}: {error}", path.display()))
                .ok()
        })
        .collect()
}

/// The file a directory's app starts at: `app.luau` or `main.luau`, or a file itself.
pub fn entry_of(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_owned());
    }
    for name in ["video.luau", "app.luau", "main.luau", "init.luau"] {
        let file = path.join(name);
        if file.is_file() {
            return Ok(file);
        }
    }
    Err(format!(
        "{} has no video.luau or app.luau to start at",
        path.display()
    ))
}

type Listener = Rc<dyn Fn(&[Composition])>;

/// What the script tells a stage's host as it runs.
#[derive(Default)]
pub(crate) struct Told {
    compositions: Vec<Composition>,
    tracks: Vec<Track>,
    audio: Vec<AudioClip>,
    holds: BTreeMap<String, String>,
    /// Video files shown, by id.
    clips: BTreeMap<String, crate::clip::Clip>,
    /// Told each time its script declares its compositions.
    listening: Option<Listener>,
}

/// A script's host, its video's states and what it tells kept.
struct Hosted {
    host: ScriptHost,
    frame: Shared,
    told: Rc<RefCell<Told>>,
}
fn host(
    runtime: &zgx::reactive::Runtime,
    entry: &Path,
    selected: Option<&str>,
    input: &Json,
    scale: f32,
) -> Result<Hosted, String> {
    let (video, fx) =
        packages().map_err(|error| format!("writing zgx-video's scripts: {error}"))?;
    let base = entry.parent().map(Path::to_owned).unwrap_or_default();
    let host = ScriptHost::new(runtime, entry)
        .package("video", video)
        .package("fx", fx);
    host.lint_strip(false);
    let told: Rc<RefCell<Told>> = Rc::default();
    let setting = json!({ "selected": selected, "input": input }).to_string();
    let _ = host.shared(
        "zgx.video.host",
        Value::from_json(&setting).unwrap_or_default(),
    );
    let frame = host.shared("zgx.video.frame", 0.);
    let heard = told.clone();
    host.action("zgx.video.compositions", move |list| {
        let list = json_of(&list);
        let list: Vec<Composition> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Composition::from_json)
            .collect();
        let listening = {
            let mut told = heard.borrow_mut();
            told.compositions = list.clone();
            told.listening.clone()
        };
        if let Some(listening) = listening {
            listening(&list);
        }
    });
    let heard = told.clone();
    host.action("zgx.video.sequence", move |sequence| {
        let sequence = json_of(&sequence);
        let track = Track {
            name: sequence["name"].as_str().unwrap_or("Sequence").to_owned(),
            from: sequence["from"].as_f64().unwrap_or(0.).max(0.) as u64,
            duration: sequence["duration"]
                .as_f64()
                .filter(|frames| *frames >= 0.)
                .map(|frames| frames as u64),
        };
        let tracks = &mut heard.borrow_mut().tracks;
        if !tracks.contains(&track) {
            tracks.push(track);
        }
    });
    let heard = told.clone();
    host.action("zgx.video.audio", move |clip| {
        let clip = json_of(&clip);
        let frames = |key: &str| clip[key].as_f64().unwrap_or(0.).max(0.) as u64;
        let clip = AudioClip {
            src: base.join(clip["src"].as_str().unwrap_or_default()),
            from: frames("from"),
            duration: clip["duration"]
                .as_f64()
                .filter(|frames| *frames >= 0.)
                .map(|frames| frames as u64),
            trim: frames("trim"),
            volume: clip["volume"].as_f64().unwrap_or(1.),
            fade_in: frames("fade_in"),
            fade_out: frames("fade_out"),
            looped: clip["loop"].as_bool().unwrap_or(false),
            optional: false,
            volumes: clip["volumes"].as_array().map(|volumes| {
                volumes
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(1.) as f32)
                    .collect()
            }),
        };
        let audio = &mut heard.borrow_mut().audio;
        if !audio.contains(&clip) {
            audio.push(clip);
        }
    });
    // A caption file's cues (`video.captions_of`).
    let caption_dir = entry.parent().map(Path::to_owned).unwrap_or_default();
    host.function("zgx.video.captions", move |arg| {
        let arg = json_of(&arg);
        let src = caption_dir.join(arg["src"].as_str().unwrap_or_default());
        let answer = match std::fs::read_to_string(&src) {
            Ok(text) => json!(
                crate::captions::parse(&text)
                    .into_iter()
                    .map(|cue| json!({ "start": cue.start, "end": cue.end, "text": cue.text }))
                    .collect::<Vec<_>>()
            ),
            Err(error) => json!({ "error": format!("{}: {error}", src.display()) }),
        };
        Value::from_json(&answer.to_string()).unwrap_or_default()
    });
    let heard = told.clone();
    let base_dir = entry.parent().map(Path::to_owned).unwrap_or_default();
    host.action("zgx.video.clip", move |clip| {
        let clip = json_of(&clip);
        let src = base_dir.join(clip["src"].as_str().unwrap_or_default());
        let id = clip["id"].as_str().unwrap_or_default().to_owned();
        let frames = |key: &str| clip[key].as_f64().filter(|frames| *frames >= 0.);
        let scale = f64::from(scale);
        let size = match (clip["w"].as_f64(), clip["h"].as_f64()) {
            (Some(w), Some(h)) => ((w * scale).round() as u32, (h * scale).round() as u32),
            _ => crate::clip::size_of(&src).unwrap_or((640, 360)),
        };
        let size = (size.0.max(2), size.1.max(2));
        let placed = crate::clip::Clip {
            id: id.clone(),
            src: src.clone(),
            from: frames("from").unwrap_or(0.),
            duration: frames("duration"),
            trim: frames("trim").unwrap_or(0.),
            size,
            looped: clip["loop"].as_bool().unwrap_or(false),
        };
        let volume = clip["volume"].as_f64().unwrap_or(1.);
        let mut told = heard.borrow_mut();
        if volume > 0. && !told.clips.contains_key(&id) {
            let sound = AudioClip {
                src,
                from: placed.from.max(0.) as u64,
                duration: placed.duration.map(|frames| frames as u64),
                trim: placed.trim as u64,
                volume,
                fade_in: 0,
                fade_out: 0,
                looped: placed.looped,
                optional: true,
                volumes: None,
            };
            if !told.audio.contains(&sound) {
                told.audio.push(sound);
            }
        }
        told.clips.insert(id, placed);
    });
    // A sound's spectrum, loudness and waveform at a moment
    // (`video.audio_data`), each file decoded once.
    let sounds: Rc<RefCell<BTreeMap<PathBuf, Rc<crate::audio::Sound>>>> = Rc::default();
    let sound_dir = entry.parent().map(Path::to_owned).unwrap_or_default();
    host.function("zgx.video.audio_data", move |arg| {
        let arg = json_of(&arg);
        let src = sound_dir.join(arg["src"].as_str().unwrap_or_default());
        let kept = sounds.borrow().get(&src).cloned();
        let sound = match kept {
            Some(sound) => sound,
            None => match crate::audio::decode(&src) {
                Ok(sound) => {
                    let sound = Rc::new(sound);
                    sounds.borrow_mut().insert(src.clone(), sound.clone());
                    sound
                }
                Err(error) => {
                    return Value::from_json(&json!({ "error": error }).to_string())
                        .unwrap_or_default();
                }
            },
        };
        let seconds = arg["seconds"].as_f64().unwrap_or(0.);
        let floor = arg["floor_db"].as_f64().unwrap_or(-70.) as f32;
        let answer = match arg["what"].as_str().unwrap_or("length") {
            "bars" => {
                let bars = crate::analysis::Bars {
                    count: arg["count"].as_u64().unwrap_or(32).clamp(1, 1024) as usize,
                    low: arg["low"].as_f64().unwrap_or(40.) as f32,
                    high: arg["high"].as_f64().unwrap_or(16_000.) as f32,
                    floor_db: floor,
                };
                json!(crate::analysis::bars(&sound, seconds, bars))
            }
            "level" => json!(crate::analysis::level(&sound, seconds, floor)),
            "waveform" => json!(crate::analysis::waveform(
                &sound,
                seconds,
                arg["span"].as_f64().unwrap_or(0.1),
                arg["count"].as_u64().unwrap_or(64).clamp(1, 4096) as usize,
            )),
            _ => json!(sound.0.len() as f64 / 2. / f64::from(crate::audio::SAMPLE_RATE)),
        };
        Value::from_json(&answer.to_string()).unwrap_or_default()
    });
    let heard = told.clone();
    host.action("zgx.video.hold", move |hold| {
        let hold = json_of(&hold);
        heard.borrow_mut().holds.insert(
            hold["id"].as_str().unwrap_or_default().to_owned(),
            hold["label"].as_str().unwrap_or_default().to_owned(),
        );
    });
    let heard = told.clone();
    host.action("zgx.video.release", move |hold| {
        let hold = json_of(&hold);
        heard
            .borrow_mut()
            .holds
            .remove(hold["id"].as_str().unwrap_or_default());
    });
    Ok(Hosted { host, frame, told })
}
fn json_of(value: &Value) -> Json {
    serde_json::from_str(&value.to_json()).unwrap_or(Json::Null)
}

/// The compositions the script at `path` declares, each given `input` if it's
/// the one `selected`. Nothing is mounted or drawn.
pub fn compositions(
    path: &Path,
    selected: Option<&str>,
    input: &Json,
) -> Result<Vec<Composition>, String> {
    let entry = entry_of(path)?;
    let ui = zgx::widgets::Ui::new(1., 1.);
    let hosted = host(&ui.runtime, &entry, selected, input, 1.)?;
    hosted.host.reload().map_err(|error| error.to_string())?;
    let list = std::mem::take(&mut hosted.told.borrow_mut().compositions);
    if list.is_empty() {
        return Err(format!(
            "{} declares no compositions: it should return video.compositions {{ … }}",
            entry.display()
        ));
    }
    Ok(list)
}

/// Follows a script as it's saved: it's loaded again (showing nothing) when a
/// file of it changes, which the system says: nothing polls.
pub struct ScriptWatch {
    _ui: zgx::widgets::Ui,
    host: ScriptHost,
    told: Rc<RefCell<Told>>,
}
impl ScriptWatch {
    /// A watch of the script at `path`. `listed` hears its compositions as each
    /// load declares them, `failed` a load's error.
    pub fn new(
        path: &Path,
        listed: impl Fn(&[Composition]) + 'static,
        failed: impl Fn(&str) + 'static,
    ) -> Result<Self, String> {
        let entry = entry_of(path)?;
        let ui = zgx::widgets::Ui::new(1., 1.);
        let Hosted { host, told, .. } = self::host(&ui.runtime, &entry, None, &json!({}), 1.)?;
        told.borrow_mut().listening = Some(Rc::new(listed));
        host.on_error(failed);
        if let Err(error) = host.reload() {
            eprintln!("{error}");
        }
        Ok(Self {
            _ui: ui,
            host,
            told,
        })
    }
    /// Run for as long as it should follow: on a window's tasks.
    pub fn follow(&self) -> impl std::future::Future<Output = ()> + 'static {
        self.host.check_on_reload(false);
        self.host.watch(std::time::Duration::from_millis(250))
    }
    /// What the script last declared.
    pub fn compositions(&self) -> Vec<Composition> {
        self.told.borrow().compositions.clone()
    }
}

/// One composition of a script on a [`Stage`] of its own: step it and draw it.
pub struct ScriptStage {
    /// What it plays on.
    pub film: Film,
    host: ScriptHost,
    told: Rc<RefCell<Told>>,
    composition: Composition,
}
impl std::ops::Deref for ScriptStage {
    type Target = Film;
    fn deref(&self) -> &Film {
        &self.film
    }
}
impl std::ops::DerefMut for ScriptStage {
    fn deref_mut(&mut self) -> &mut Film {
        &mut self.film
    }
}
impl ScriptStage {
    /// Composition `id` (the first, if `None`) of the script at `path`, given
    /// `input` over its props, `scale` pixels a point, its background `background`.
    pub fn new(
        path: &Path,
        id: Option<&str>,
        input: &Json,
        scale: f32,
        background: zgx::scene::Color,
        timing: Timing,
    ) -> Result<Self, String> {
        let list = compositions(path, id, input)?;
        let composition = match id {
            Some(id) => list.iter().find(|comp| comp.id == id).ok_or_else(|| {
                let names: Vec<&str> = list.iter().map(|comp| comp.id.as_str()).collect();
                format!("no composition `{id}`; there are: {}", names.join(", "))
            })?,
            None => &list[0],
        }
        .clone();
        Self::with(path, composition, input, scale, background, timing)
    }
    /// `composition`, as [`compositions`] listed it.
    pub fn with(
        path: &Path,
        composition: Composition,
        input: &Json,
        scale: f32,
        background: zgx::scene::Color,
        timing: Timing,
    ) -> Result<Self, String> {
        let entry = entry_of(path)?;
        let ticks = Ticks::of(timing, composition.fps);
        let options = StageOptions {
            width: composition.width,
            height: composition.height,
            fps: ticks.stage_fps(),
            scale,
            background,
            fonts: fonts_beside(&entry),
        };
        let mut hosted = None;
        let id = composition.id.clone();
        let stage = Stage::new(options, |setting: Setting<'_>| {
            let made = host(setting.runtime, &entry, Some(&id), input, scale)?;
            made.host.reload().map_err(|error| error.to_string())?;
            let view = made.host.view();
            hosted = Some(made);
            Ok(view)
        })?;
        let Hosted { host, frame, told } = hosted.expect("built with the stage");
        // What it read at its load is the composition's real measure, once its
        // props are in.
        let composition = told
            .borrow()
            .compositions
            .iter()
            .find(|comp| comp.id == id)
            .cloned()
            .unwrap_or(composition);
        let errors = host.error();
        let holding = told.clone();
        let showing = told.clone();
        let shown_host = host.clone();
        let decoders: RefCell<BTreeMap<String, (crate::clip::Decoder, Option<u64>)>> =
            RefCell::default();
        let fps = composition.fps;
        let frame_now = frame.clone();
        let film = Film::new(
            stage,
            ticks,
            composition.fps,
            composition.duration,
            Player {
                set_frame: Box::new(move |at| {
                    frame.set(Value::Num(at));
                }),
                check: Box::new(move || match errors.with_untracked(Clone::clone) {
                    Some(error) => Err(error),
                    None => Ok(()),
                }),
                holds: Box::new(move || holding.borrow().holds.values().cloned().collect()),
                after_step: Box::new(move || {
                    let clips: Vec<crate::clip::Clip> =
                        showing.borrow().clips.values().cloned().collect();
                    if clips.is_empty() {
                        return Ok(false);
                    }
                    let at = frame_now.with_untracked(|value| match value {
                        Value::Num(at) => *at,
                        _ => 0.,
                    });
                    let mut decoders = decoders.borrow_mut();
                    let mut changed = false;
                    for clip in clips {
                        let local = at - clip.from;
                        if local < 0. || clip.duration.is_some_and(|lasts| local >= lasts) {
                            continue;
                        }
                        let entry = decoders.entry(clip.id.clone()).or_insert_with(|| {
                            (crate::clip::Decoder::new(clip.clone(), fps), None)
                        });
                        if *entry.0.clip() != clip {
                            *entry = (crate::clip::Decoder::new(clip.clone(), fps), None);
                        }
                        let wanted = (clip.trim + local).floor().max(0.) as u64;
                        if entry.1 == Some(wanted) {
                            continue;
                        }
                        let pixels = entry.0.frame(wanted)?;
                        let image =
                            zgx::image::ImageData::new(clip.size.0, clip.size.1, pixels.to_vec())
                                .map_err(str::to_owned)?;
                        shown_host.image(&clip.id, Some(std::sync::Arc::new(image)));
                        entry.1 = Some(wanted);
                        changed = true;
                    }
                    Ok(changed)
                }),
            },
        )?;
        Ok(Self {
            film,
            host,
            told,
            composition,
        })
    }
    /// The composition it plays.
    pub fn composition(&self) -> &Composition {
        &self.composition
    }
    /// The host running its script.
    pub fn host(&self) -> &ScriptHost {
        &self.host
    }
    /// The sequences it has shown so far.
    pub fn tracks(&self) -> Vec<Track> {
        self.told.borrow().tracks.clone()
    }
    /// The sounds it has placed so far.
    pub fn audio(&self) -> Vec<AudioClip> {
        self.told.borrow().audio.clone()
    }
}
