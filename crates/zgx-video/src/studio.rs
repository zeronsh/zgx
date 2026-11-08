//! `zgx video studio`: a composition played live, in an app written with zgx
//! (`studio/app.luau`). Its frames come from a thread of their own, on that
//! thread's virtual clock, through the same film a render uses: what it shows
//! is what a render writes. Nothing runs while nothing is asked: the thread
//! waits for a message, unless it's playing.
use crate::player::{Event, Player};
use crate::render::{Progress, RenderOptions, render};
use crate::script::{Composition, ScriptWatch, compositions, entry_of};
use serde_json::{Value as Json, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// A render asked for: waiting its turn, running, or over.
struct Job {
    /// Its number, from 1, as the app names it.
    number: u64,
    options: RenderOptions,
    state: &'static str,
    done: u64,
    total: u64,
    said: String,
    cancel: Arc<AtomicBool>,
}
/// The renders asked for, run one after another.
#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    next: u64,
    /// A job is running: a new one waits for it.
    working: bool,
}
/// How many finished renders the app is shown.
const KEPT_JOBS: usize = 12;

/// The app's host: what its script asks and is told.
pub struct Studio {
    player: Rc<Player>,
    state: Shared,
    host: ScriptHost,
    path: PathBuf,
    queue: RefCell<Queue>,
    shown: RefCell<Option<Arc<[u8]>>>,
}

/// Attach the studio for the script at `path` to `host`, which runs the app:
/// `spawn` runs what it follows on the host's main loop, `wake` being safe to
/// call from anywhere.
pub fn attach(host: &ScriptHost, path: &Path, scale: f32) -> Result<Rc<Studio>, String> {
    let path = path.to_owned();
    entry_of(&path)?;
    let state = host.shared("studio", state_json(&json!({})));
    // The studio follows the player's events into its own state; the
    // player's signals, which nothing here reads, keep a runtime of their own.
    let player = Player::new(&zgx::reactive::Runtime::new(), &path, scale)?;
    let studio = Rc::new(Studio {
        player,
        state,
        host: host.clone(),
        path,
        queue: RefCell::default(),
        shown: RefCell::new(None),
    });
    // What the player tells comes through on the main loop, when the app's
    // script starts listening: `ui.call("studio.listen")`.
    let listening = studio.clone();
    host.async_action("studio.listen", move |_| {
        let studio = listening.clone();
        let heard = studio.clone();
        let listen = studio.player.listen(move |event| heard.heard(event));
        async move {
            listen.await;
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
        this.player.open(&id, props, frame);
    });
    let this = studio.clone();
    host.action("studio.seek", move |arg| {
        let frame = json_of(&arg).as_f64().unwrap_or(0.).max(0.) as u64;
        this.player.seek(frame);
    });
    let this = studio.clone();
    host.action("studio.play", move |arg| {
        let on = json_of(&arg).as_bool().unwrap_or(false);
        this.merge(json!({ "playing": on }));
        this.player.play(on);
    });
    let this = studio.clone();
    host.action("studio.loop", move |arg| {
        let on = json_of(&arg).as_bool().unwrap_or(true);
        this.merge(json!({ "looping": on }));
        this.player.looping(on);
    });
    let this = studio.clone();
    host.async_action("studio.render", move |arg| {
        let (studio, arg) = (this.clone(), json_of(&arg));
        async move {
            let number = studio.enqueue(arg);
            studio.work().await;
            Ok(Value::Num(number as f64))
        }
    });
    let this = studio.clone();
    host.action("studio.cancel", move |arg| {
        let number = json_of(&arg).as_u64();
        this.cancel(number);
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
        self.player.reload();
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
    fn heard(&self, event: &Event) {
        match event {
            Event::Opened { composition } => {
                self.merge(json!({ "open": composition_json(composition), "tracks": [] }));
            }
            Event::Tracks(tracks) => {
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
            Event::Stopped => self.merge(json!({ "playing": false })),
            Event::Error(error) => self.merge(json!({ "error": error })),
            Event::Frame {
                frame,
                width,
                height,
                pixels,
            } => {
                *self.shown.borrow_mut() = Some(pixels.clone());
                if let Ok(image) = ImageData::new(*width, *height, pixels.clone()) {
                    self.host.image("preview", Some(Arc::new(image)));
                }
                self.merge(json!({ "frame": frame, "size": [width, height], "error": null }));
            }
        }
    }
    /// Queue a render of `arg`'s composition (`id`, `props`, `output`,
    /// `scale`); its number.
    fn enqueue(&self, arg: Json) -> u64 {
        let options = RenderOptions {
            script: self.path.clone(),
            composition: arg["id"].as_str().map(str::to_owned),
            output: arg["output"].as_str().unwrap_or("out.mp4").into(),
            props: arg.get("props").cloned().unwrap_or(json!({})),
            scale: arg["scale"].as_f64().unwrap_or(1.) as f32,
            ..Default::default()
        };
        let number = {
            let mut queue = self.queue.borrow_mut();
            queue.next += 1;
            let number = queue.next;
            queue.jobs.push(Job {
                number,
                options,
                state: "queued",
                done: 0,
                total: 0,
                said: String::new(),
                cancel: Arc::default(),
            });
            // What's over goes, past the latest few.
            let over = queue.jobs.iter().filter(|job| is_over(job.state)).count();
            let mut drop = over.saturating_sub(KEPT_JOBS);
            queue.jobs.retain(|job| {
                let goes = drop > 0 && is_over(job.state);
                drop -= usize::from(goes);
                !goes
            });
            number
        };
        self.publish();
        number
    }
    /// Stop render `number` (the running one, if `None`): a queued one never
    /// starts, a running one stops at its next frame.
    fn cancel(&self, number: Option<u64>) {
        let mut queue = self.queue.borrow_mut();
        for job in &mut queue.jobs {
            let chosen = number.map_or(job.state == "running", |number| job.number == number);
            if !chosen {
                continue;
            }
            match job.state {
                "queued" => job.state = "stopped",
                "running" => job.cancel.store(true, Ordering::Relaxed),
                _ => {}
            }
        }
        drop(queue);
        self.publish();
    }
    fn publish(&self) {
        let jobs: Vec<Json> = self
            .queue
            .borrow()
            .jobs
            .iter()
            .map(|job| {
                json!({
                    "number": job.number,
                    "id": job.options.composition,
                    "output": job.options.output,
                    "state": job.state,
                    "done": job.done,
                    "total": job.total,
                    "said": job.said,
                })
            })
            .collect();
        self.merge(json!({ "renders": jobs }));
    }
    /// Run what's queued, one render after another, unless a worker already
    /// is: the first caller works until the queue is empty.
    async fn work(self: Rc<Self>) {
        {
            let mut queue = self.queue.borrow_mut();
            if queue.working {
                return;
            }
            queue.working = true;
        }
        loop {
            let next = {
                let mut queue = self.queue.borrow_mut();
                let job = queue.jobs.iter_mut().find(|job| job.state == "queued");
                job.map(|job| {
                    job.state = "running";
                    (job.number, job.options.clone(), job.cancel.clone())
                })
            };
            let Some((number, options, cancel)) = next else {
                break;
            };
            self.publish();
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
                let heard = heard.recv().await;
                let mut queue = self.queue.borrow_mut();
                let Some(job) = queue.jobs.iter_mut().find(|job| job.number == number) else {
                    break;
                };
                match heard {
                    Some(Ok(progress)) => {
                        (job.done, job.total) = (progress.done, progress.total);
                    }
                    Some(Err(over)) => {
                        (job.state, job.said) = match over {
                            Ok(said) => ("done", said),
                            Err(error) if error == "stopped" => ("stopped", String::new()),
                            Err(error) => ("failed", error),
                        };
                    }
                    None => job.state = "failed",
                }
                let over = is_over(job.state);
                drop(queue);
                self.publish();
                if over {
                    break;
                }
            }
        }
        self.queue.borrow_mut().working = false;
    }
}

fn is_over(state: &str) -> bool {
    matches!(state, "done" | "failed" | "stopped")
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
            ..Default::default()
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
            studio.player.seek(frame);
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
