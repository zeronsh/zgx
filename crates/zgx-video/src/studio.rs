//! `zgx video studio`: a composition played live, in an app written with zgx
//! (`studio/app.luau`). Its frames come from a thread of their own, on that
//! thread's virtual clock, through the same film a render uses: what it shows
//! is what a render writes. Nothing runs while nothing is asked: the thread
//! waits for a message, unless it's playing.
use crate::film::Timing;
use crate::render::{Progress, RenderOptions, render};
use crate::script::{Composition, ScriptStage, ScriptWatch, compositions, entry_of};
use serde_json::{Value as Json, json};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};
use zgx::image::ImageData;
use zgx::scene::Color;
use zgx_script::{ScriptHost, Shared, Value};

/// The source of the app, for its host to run.
const STUDIO_LUAU: &str = include_str!("../studio/app.luau");

/// Write the app where a host can read it; the file it starts at.
pub fn app_file() -> std::io::Result<PathBuf> {
    static WRITTEN: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    WRITTEN
        .get_or_init(|| write_app().map_err(|error| error.to_string()))
        .clone()
        .map_err(std::io::Error::other)
}
fn write_app() -> std::io::Result<PathBuf> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    STUDIO_LUAU.hash(&mut hasher);
    let dir = std::env::temp_dir().join(format!("zgx-video-studio-{:016x}", hasher.finish()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("app.luau");
    if !std::fs::read_to_string(&file).is_ok_and(|on_disk| on_disk == STUDIO_LUAU) {
        let partial = dir.join(format!("app.{}.tmp", std::process::id()));
        std::fs::write(&partial, STUDIO_LUAU)?;
        std::fs::rename(&partial, &file)?;
    }
    Ok(file)
}

/// What the app asks of the thread making its frames.
enum Ask {
    /// Show this composition, with these props, from this frame.
    Open(String, Json, u64),
    Seek(u64),
    Play(bool),
    Loop(bool),
    /// The script changed: build it again, staying at the frame it's at.
    Reload,
    Quit,
}

/// What that thread tells.
enum Told {
    Frame {
        frame: u64,
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
    },
    Opened {
        composition: Box<Composition>,
    },
    Tracks(Vec<crate::script::Track>),
    Stopped,
    Error(String),
}

/// How many bytes of drawn frames the thread keeps, for scrubbing back without
/// playing the composition from its start.
const CACHE_BYTES: usize = 512 << 20;

/// Frames drawn for this script, as they were.
#[derive(Default)]
struct Cache {
    frames: HashMap<u64, Arc<[u8]>>,
    order: VecDeque<u64>,
    bytes: usize,
}
impl Cache {
    fn get(&mut self, frame: u64) -> Option<Arc<[u8]>> {
        self.frames.get(&frame).cloned()
    }
    fn put(&mut self, frame: u64, pixels: Arc<[u8]>) {
        self.bytes += pixels.len();
        if self.frames.insert(frame, pixels).is_none() {
            self.order.push_back(frame);
        }
        while self.bytes > CACHE_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.frames.remove(&oldest) {
                self.bytes -= gone.len();
            }
        }
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// What's playing on the thread, and how to get to any frame of it.
struct Preview {
    path: PathBuf,
    scale: f32,
    composition: Option<Composition>,
    props: Json,
    stage: Option<ScriptStage>,
    cache: Cache,
    tell: Sender<Told>,
    wake: Rc<dyn Fn()>,
    tracks: usize,
}
impl Preview {
    fn say(&self, told: Told) {
        let _ = self.tell.send(told);
        (self.wake)();
    }
    fn build(&mut self) -> Result<(), String> {
        self.stage = None;
        let id = self.composition.as_ref().map(|comp| comp.id.clone());
        let list = compositions(&self.path, id.as_deref(), &self.props)?;
        let composition = id
            .and_then(|id| list.iter().find(|comp| comp.id == id).cloned())
            .unwrap_or_else(|| list[0].clone());
        // The preview is drawn no larger than a window shows it.
        self.stage = Some(ScriptStage::with(
            &self.path,
            composition.clone(),
            &self.props,
            self.scale,
            Color(0, 0, 0, 0),
            Timing::default(),
        )?);
        let same = self.composition.as_ref() == Some(&composition);
        self.composition = Some(composition.clone());
        self.tracks = 0;
        if !same {
            self.say(Told::Opened {
                composition: Box::new(composition),
            });
        }
        Ok(())
    }
    /// Show `frame`: drawn before, or by moving there, from the start if it's
    /// behind where the stage stands.
    fn show(&mut self, frame: u64) -> Result<(), String> {
        if self.stage.is_none() {
            self.build()?;
        }
        let (width, height) = self.stage.as_ref().map_or((1, 1), |s| s.stage.pixel_size());
        if let Some(pixels) = self.cache.get(frame) {
            self.say(Told::Frame {
                frame,
                width,
                height,
                pixels,
            });
            return Ok(());
        }
        if self
            .stage
            .as_ref()
            .is_some_and(|stage| stage.output_frame() > frame)
        {
            self.build()?;
        }
        let stage = self.stage.as_mut().expect("built");
        let mut pixels = Vec::new();
        stage.draw_output(frame, &mut pixels, Duration::from_secs(30))?;
        let tracks = stage.tracks();
        if tracks.len() != self.tracks {
            self.tracks = tracks.len();
            self.say(Told::Tracks(tracks));
        }
        let pixels: Arc<[u8]> = pixels.into();
        self.cache.put(frame, pixels.clone());
        self.say(Told::Frame {
            frame,
            width,
            height,
            pixels,
        });
        Ok(())
    }
    /// Move there without drawing: playing, behind time.
    fn last_frame(&self) -> u64 {
        self.composition
            .as_ref()
            .map_or(0, |comp| comp.duration.saturating_sub(1))
    }
    fn fps(&self) -> f64 {
        self.composition.as_ref().map_or(30., |comp| comp.fps)
    }
}

/// The thread's loop. Playing, it keeps to real time, drawing the frame due
/// now; else it sleeps until it's asked something.
fn run(
    path: PathBuf,
    scale: f32,
    asks: Receiver<Ask>,
    tell: Sender<Told>,
    wake: Box<dyn Fn() + Send>,
) {
    let wake: Rc<dyn Fn()> = Rc::from(wake as Box<dyn Fn()>);
    let mut preview = Preview {
        path,
        scale,
        composition: None,
        props: json!({}),
        stage: None,
        cache: Cache::default(),
        tell,
        wake,
        tracks: 0,
    };
    let mut at = 0u64;
    let mut playing: Option<(Instant, u64)> = None;
    let mut looping = true;
    loop {
        // Until the next frame is due, or for ever: an idle preview waits.
        let ask = match playing {
            Some((started, from)) => {
                let due = started + Duration::from_secs_f64((at + 1 - from) as f64 / preview.fps());
                match asks.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(ask) => Some(ask),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match asks.recv() {
                Ok(ask) => Some(ask),
                Err(_) => return,
            },
        };
        // The latest of what's waiting: scrubbing runs ahead of drawing.
        let mut asked = ask.into_iter().collect::<Vec<_>>();
        asked.extend(asks.try_iter());
        let mut seek = None;
        let mut result = Ok(());
        for ask in asked {
            match ask {
                Ask::Quit => return,
                Ask::Open(id, props, frame) => {
                    if preview
                        .composition
                        .as_ref()
                        .is_none_or(|comp| comp.id != id)
                        || preview.props != props
                    {
                        preview.cache.clear();
                        preview.props = props;
                        let list = compositions(&preview.path, Some(&id), &preview.props);
                        match list.map(|list| list.into_iter().find(|comp| comp.id == id)) {
                            Ok(Some(comp)) => {
                                let same =
                                    preview.composition.as_ref().is_some_and(|was| *was == comp);
                                preview.composition = Some(comp.clone());
                                preview.stage = None;
                                if !same {
                                    preview.say(Told::Opened {
                                        composition: Box::new(comp),
                                    });
                                }
                                result = preview.build();
                            }
                            Ok(None) => result = Err(format!("no composition `{id}`")),
                            Err(error) => result = Err(error),
                        }
                    }
                    seek = Some(frame);
                }
                Ask::Seek(frame) => {
                    seek = Some(frame);
                    playing = playing.map(|_| (Instant::now(), frame));
                }
                Ask::Play(on) => {
                    if on {
                        if at >= preview.last_frame() {
                            at = 0;
                            seek = Some(0);
                        }
                        playing = Some((Instant::now(), at));
                    } else {
                        playing = None;
                    }
                }
                Ask::Loop(on) => looping = on,
                Ask::Reload => {
                    preview.cache.clear();
                    result = preview.build();
                    seek = Some(at);
                }
            }
        }
        if result.is_ok() {
            if let Some((started, from)) = playing.filter(|_| seek.is_none()) {
                // Behind time, it skips: real speed comes first.
                let due = from + (started.elapsed().as_secs_f64() * preview.fps()) as u64;
                let last = preview.last_frame();
                if due > last {
                    if looping {
                        playing = Some((Instant::now(), 0));
                        seek = Some(0);
                    } else {
                        playing = None;
                        seek = Some(last);
                        preview.say(Told::Stopped);
                    }
                } else {
                    seek = Some(due.max(at + 1));
                }
            }
            if let Some(frame) = seek {
                at = frame.min(
                    preview
                        .last_frame()
                        .max(frame * u64::from(preview.composition.is_none())),
                );
                result = preview.show(at);
            }
        }
        if let Err(error) = result {
            playing = None;
            preview.say(Told::Stopped);
            preview.say(Told::Error(error));
        }
    }
}

/// A render going on behind the app.
struct Rendering {
    cancel: Arc<AtomicBool>,
}

/// The app's host: what its script asks and is told.
pub struct Studio {
    asks: Sender<Ask>,
    state: Shared,
    host: ScriptHost,
    path: PathBuf,
    rendering: RefCell<Option<Rendering>>,
    shown: RefCell<Option<Arc<[u8]>>>,
}

/// Attach the studio for the script at `path` to `host`, which runs the app:
/// `spawn` runs what it follows on the host's main loop, `wake` being safe to
/// call from anywhere.
pub fn attach(host: &ScriptHost, path: &Path, scale: f32) -> Result<Rc<Studio>, String> {
    let path = path.to_owned();
    entry_of(&path)?;
    let (asks, asked) = channel();
    let (tell, told) = channel::<Told>();
    let state = host.shared("studio", state_json(&json!({})));
    let (wake_up, woken) = zgx::task::latest::<()>();
    let thread_path = path.clone();
    std::thread::Builder::new()
        .name("zgx-video preview".into())
        .spawn(move || {
            run(
                thread_path,
                scale,
                asked,
                tell,
                Box::new(move || {
                    wake_up.send(());
                }),
            );
        })
        .map_err(|error| error.to_string())?;
    let studio = Rc::new(Studio {
        asks,
        state,
        host: host.clone(),
        path,
        rendering: RefCell::new(None),
        shown: RefCell::new(None),
    });
    // What the thread tells comes through on the main loop, when the app's
    // script starts listening: `ui.call("studio.listen")`.
    let listening = studio.clone();
    let told = Rc::new(told);
    let woken = RefCell::new(Some(woken));
    host.async_action("studio.listen", move |_| {
        let (studio, told) = (listening.clone(), told.clone());
        let woken = woken.borrow_mut().take();
        async move {
            let Some(mut woken) = woken else {
                return Ok(Value::Nil);
            };
            while woken.recv().await.is_some() {
                // The latest frame only: an image each frame is enough to show.
                let mut frame = None;
                for message in told.try_iter() {
                    match message {
                        Told::Frame {
                            frame: at,
                            width,
                            height,
                            pixels,
                        } => frame = Some((at, width, height, pixels)),
                        other => studio.heard(other),
                    }
                }
                if let Some((at, width, height, pixels)) = frame {
                    *studio.shown.borrow_mut() = Some(pixels.clone());
                    if let Ok(image) = ImageData::new(width, height, pixels) {
                        studio.host.image("preview", Some(Arc::new(image)));
                    }
                    studio.merge(json!({ "frame": at, "size": [width, height], "error": null }));
                }
            }
            Ok(Value::Nil)
        }
    });
    // Saved, the script is listed and its preview built again.
    let this = studio.clone();
    host.async_action("studio.watch", move |_| {
        let studio = this.clone();
        async move {
            let first = Rc::new(std::cell::Cell::new(true));
            let listening = studio.clone();
            let failing = studio.clone();
            let watch = ScriptWatch::new(
                &studio.path,
                move |list| {
                    let listed: Vec<Json> = list.iter().map(composition_json).collect();
                    listening.merge(json!({ "compositions": listed, "error": null }));
                    if !first.replace(false) {
                        listening.reload();
                    }
                },
                move |error| failing.merge(json!({ "error": error })),
            )?;
            watch.follow().await;
            drop(watch);
            Ok(Value::Nil)
        }
    });
    let this = studio.clone();
    host.action("studio.open", move |arg| {
        let arg = json_of(&arg);
        let id = arg["id"].as_str().unwrap_or_default().to_owned();
        let props = arg.get("props").cloned().unwrap_or(json!({}));
        let frame = arg["frame"].as_f64().unwrap_or(0.) as u64;
        let _ = this.asks.send(Ask::Open(id, props, frame));
    });
    let this = studio.clone();
    host.action("studio.seek", move |arg| {
        let frame = json_of(&arg).as_f64().unwrap_or(0.).max(0.) as u64;
        let _ = this.asks.send(Ask::Seek(frame));
    });
    let this = studio.clone();
    host.action("studio.play", move |arg| {
        let on = json_of(&arg).as_bool().unwrap_or(false);
        this.merge(json!({ "playing": on }));
        let _ = this.asks.send(Ask::Play(on));
    });
    let this = studio.clone();
    host.action("studio.loop", move |arg| {
        let on = json_of(&arg).as_bool().unwrap_or(true);
        this.merge(json!({ "looping": on }));
        let _ = this.asks.send(Ask::Loop(on));
    });
    let this = studio.clone();
    host.async_action("studio.render", move |arg| {
        let (studio, arg) = (this.clone(), json_of(&arg));
        async move {
            studio.start_render(arg).await;
            Ok(Value::Nil)
        }
    });
    let this = studio.clone();
    host.action("studio.cancel", move |_| {
        if let Some(rendering) = &*this.rendering.borrow() {
            rendering.cancel.store(true, Ordering::Relaxed);
        }
    });
    Ok(studio)
}

fn json_of(value: &Value) -> Json {
    serde_json::from_str(&value.to_json()).unwrap_or(Json::Null)
}
fn state_json(value: &Json) -> Value {
    Value::from_json(&value.to_string()).unwrap_or_default()
}

impl Studio {
    /// The app's state, as it reads it.
    pub fn state(&self) -> Json {
        json_of(&self.state.with_untracked(Clone::clone))
    }
    /// The frame it shows, RGBA, premultiplied.
    pub fn preview_pixels(&self) -> Option<Arc<[u8]>> {
        self.shown.borrow().clone()
    }
    /// The script's compositions, into the app's state; the first opened, if
    /// none is. Call it on each load of the script.
    pub fn refresh(&self, input: &Json) {
        match compositions(&self.path, None, input) {
            Ok(list) => {
                let listed: Vec<Json> = list.iter().map(composition_json).collect();
                self.merge(json!({ "compositions": listed, "path": self.path, "error": null }));
            }
            Err(error) => self.merge(json!({ "error": error, "path": self.path })),
        }
    }
    /// The script changed.
    pub fn reload(&self) {
        let _ = self.asks.send(Ask::Reload);
    }
    fn merge(&self, fields: Json) {
        let was = json_of(&self.state.with_untracked(Clone::clone));
        let mut merged = if was.is_object() { was } else { json!({}) };
        if let (Some(merged), Some(fields)) = (merged.as_object_mut(), fields.as_object()) {
            for (key, value) in fields {
                merged.insert(key.clone(), value.clone());
            }
        }
        self.state.set(state_json(&merged));
    }
    fn heard(&self, told: Told) {
        match told {
            Told::Opened { composition } => {
                self.merge(json!({ "open": composition_json(&composition), "tracks": [] }));
            }
            Told::Tracks(tracks) => {
                let tracks: Vec<Json> = tracks
                    .iter()
                    .map(|track| {
                        json!({
                            "name": track.name,
                            "from": track.from,
                            "duration": track.duration.map_or(-1, |frames| frames as i64),
                        })
                    })
                    .collect();
                self.merge(json!({ "tracks": tracks }));
            }
            Told::Stopped => self.merge(json!({ "playing": false })),
            Told::Error(error) => self.merge(json!({ "error": error })),
            Told::Frame { .. } => {}
        }
    }
    async fn start_render(self: Rc<Self>, arg: Json) {
        if self.rendering.borrow().is_some() {
            return;
        }
        let output = arg["output"].as_str().unwrap_or("out.mp4").to_owned();
        let options = RenderOptions {
            script: self.path.clone(),
            composition: arg["id"].as_str().map(str::to_owned),
            output: output.clone().into(),
            props: arg.get("props").cloned().unwrap_or(json!({})),
            scale: arg["scale"].as_f64().unwrap_or(1.) as f32,
            ..Default::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        *self.rendering.borrow_mut() = Some(Rendering {
            cancel: cancel.clone(),
        });
        self.merge(
            json!({ "render": { "state": "running", "done": 0, "total": 0, "output": output } }),
        );
        let (tell, mut heard) = zgx::task::latest::<Result<Progress, Result<String, String>>>();
        std::thread::spawn(move || {
            let result = render(&options, |progress| {
                tell.send(Ok(progress));
                !cancel.load(Ordering::Relaxed)
            });
            tell.send(Err(match result {
                Ok(report) => Ok(format!(
                    "{} frames in {:.1} s, {:.0} a second",
                    report.frames,
                    report.elapsed.as_secs_f64(),
                    report.rate
                )),
                Err(error) => Err(error),
            }));
        });
        loop {
            match heard.recv().await {
                Some(Ok(progress)) => {
                    self.merge(json!({ "render": {
                        "state": "running", "done": progress.done, "total": progress.total, "output": output,
                    } }));
                }
                Some(Err(done)) => {
                    let (state, said) = match done {
                        Ok(said) => ("done", said),
                        Err(error) if error == "stopped" => ("stopped", String::new()),
                        Err(error) => ("failed", error),
                    };
                    self.merge(
                        json!({ "render": { "state": state, "said": said, "output": output } }),
                    );
                    break;
                }
                None => break,
            }
        }
        *self.rendering.borrow_mut() = None;
    }
}
impl Drop for Studio {
    fn drop(&mut self) {
        let _ = self.asks.send(Ask::Quit);
    }
}

fn composition_json(comp: &Composition) -> Json {
    json!({
        "id": comp.id, "w": comp.width, "h": comp.height, "fps": comp.fps,
        "duration": comp.duration, "props": comp.props, "defaults": comp.defaults, "schema": comp.schema,
    })
}

/// The studio's picture, headless: `path`'s composition at `frame`, in a stage
/// `size` points large, written as a PNG at `shot`.
pub fn screenshot(path: &Path, shot: &Path, size: (f32, f32), frame: u64) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let app = app_file().map_err(|error| error.to_string())?;
    let mut held = None;
    let mut stage = crate::stage::Stage::new(
        crate::stage::StageOptions {
            width: size.0,
            height: size.1,
            fps: 60.,
            scale: 2.,
            background: Color(14, 15, 20, 255),
        },
        |setting| {
            let host = ScriptHost::new(setting.runtime, &app);
            host.lint_strip(false);
            let studio = attach(&host, &path, 1.)?;
            studio.refresh(&json!({}));
            host.reload().map_err(|error| error.to_string())?;
            let view = host.view();
            held = Some((host, studio));
            Ok(view)
        },
    )?;
    let (host, studio) = held.expect("built with the stage");
    // Real threads make its frames: it waits for them, in real time.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut asked = false;
    loop {
        stage.step()?;
        let state = json_of(&studio.state.with_untracked(Clone::clone));
        if let Some(error) = host.error().with_untracked(Clone::clone) {
            return Err(error);
        }
        if !asked && state["open"].is_object() {
            asked = true;
            let _ = studio.asks.send(Ask::Seek(frame));
        }
        if asked && state["frame"].as_u64() == Some(frame) {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!("the preview didn't come: {state}"));
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    // Its image and what follows it, laid out and drawn.
    for _ in 0..3 {
        stage.step()?;
    }
    let mut pixels = Vec::new();
    stage.draw(&mut pixels).map_err(|error| error.to_string())?;
    let (width, height) = stage.pixel_size();
    let file =
        std::fs::File::create(shot).map_err(|error| format!("{}: {error}", shot.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())
}
