//! A composition playing live, in any zgx app: its frames made on a thread of
//! its own, on that thread's virtual clock, through the film a render uses, so
//! what plays is what a render writes. `Player::view` shows it; the studio is
//! a `Player` with a timeline and a props panel around it. Nothing runs while
//! it's neither playing nor asked for a frame: its thread waits for a message.
use crate::film::Timing;
use crate::script::{Composition, ScriptStage, compositions};
use serde_json::{Value as Json, json};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};
use zgx::image::ImageData;
use zgx::reactive::{Runtime, Signal};
use zgx::scene::Color;

/// What the app asks of the thread making its frames.
pub(crate) enum Ask {
    /// Show this composition, with these props, from this frame.
    Open(String, Json, u64),
    Seek(u64),
    Play(bool),
    Loop(bool),
    /// Whether what it draws is seen: unseen, it draws nothing.
    Watch(bool),
    /// The script changed: build it again, staying at the frame it's at.
    Reload,
    Quit,
}

/// What a player's thread tells: a frame drawn, the composition opened, the
/// sequences shown so far, playing stopped, or why it couldn't go on.
#[derive(Clone, Debug)]
pub enum Event {
    /// A frame drawn: RGBA, premultiplied.
    Frame {
        /// Which.
        frame: u64,
        /// Its width in pixels.
        width: u32,
        /// Its height.
        height: u32,
        /// Its pixels.
        pixels: Arc<[u8]>,
    },
    /// A composition opened.
    Opened {
        /// Which, as its props measure it.
        composition: Box<Composition>,
    },
    /// The sequences shown so far.
    Tracks(Vec<crate::script::Track>),
    /// Playing stopped (at its end, not looping, or for an error).
    Stopped,
    /// Why it couldn't go on.
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
    tell: Sender<Event>,
    wake: Rc<dyn Fn()>,
    tracks: usize,
}
impl Preview {
    fn say(&self, told: Event) {
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
        // No wider than a window shows it: playing keeps up.
        let scale = self.scale * (1280. / composition.width).min(1.);
        self.stage = Some(ScriptStage::with(
            &self.path,
            composition.clone(),
            &self.props,
            scale,
            Color(0, 0, 0, 0),
            Timing::default(),
        )?);
        let same = self.composition.as_ref() == Some(&composition);
        self.composition = Some(composition.clone());
        self.tracks = 0;
        if !same {
            self.say(Event::Opened {
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
            self.say(Event::Frame {
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
            self.say(Event::Tracks(tracks));
        }
        let pixels: Arc<[u8]> = pixels.into();
        self.cache.put(frame, pixels.clone());
        self.say(Event::Frame {
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
pub(crate) fn run(
    path: PathBuf,
    scale: f32,
    asks: Receiver<Ask>,
    tell: Sender<Event>,
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
    // Unseen (its box offscreen, scrolled away, its window covered), it
    // draws nothing: its clock runs on, and seen again it draws the frame
    // due then. `stale`: a frame it moved to and didn't draw.
    let mut watched = true;
    let mut stale = false;
    loop {
        // Until the next frame is due, or for ever: an idle preview waits.
        // Unseen and playing, only its end matters (not looping), or nothing.
        let wait = match playing {
            Some((started, from)) if watched => {
                Some(started + Duration::from_secs_f64((at + 1 - from) as f64 / preview.fps()))
            }
            Some((started, from)) if !looping => Some(
                started
                    + Duration::from_secs_f64(
                        (preview.last_frame() + 1).saturating_sub(from) as f64 / preview.fps(),
                    ),
            ),
            _ => None,
        };
        let ask = match wait {
            Some(due) => match asks.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(ask) => Some(ask),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
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
                                    preview.say(Event::Opened {
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
                Ask::Watch(on) => {
                    // Seen again: playing, it draws the frame due now
                    // (below); else the one it moved to unseen.
                    if on && !watched && stale && playing.is_none() {
                        seek = Some(at);
                    }
                    watched = on;
                }
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
                        // Around again, as far into it as time says (unseen
                        // for longer than a pass, past several).
                        let into = (due - last - 1) % (last + 1);
                        let back = Duration::from_secs_f64(into as f64 / preview.fps());
                        playing = Some((
                            Instant::now()
                                .checked_sub(back)
                                .unwrap_or_else(Instant::now),
                            0,
                        ));
                        seek = Some(into);
                    } else {
                        playing = None;
                        seek = Some(last);
                        preview.say(Event::Stopped);
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
                if watched {
                    result = preview.show(at);
                    stale = false;
                } else {
                    stale = true;
                }
            }
        }
        if let Err(error) = result {
            playing = None;
            preview.say(Event::Stopped);
            preview.say(Event::Error(error));
        }
    }
}

/// A composition playing live: ask it for frames, play it, give it props;
/// what it shows follows as signals and as [`Player::view`].
pub struct Player {
    asks: Sender<Ask>,
    events: RefCell<Option<(Receiver<Event>, zgx::task::LatestReceiver<()>)>>,
    /// The frame it shows.
    pub frame: Signal<u64>,
    /// Whether it plays.
    pub playing: Signal<bool>,
    /// What it opened.
    pub composition: Signal<Option<Composition>>,
    /// The sequences shown so far.
    pub tracks: Signal<Vec<crate::script::Track>>,
    /// Why it couldn't go on, until it can.
    pub error: Signal<Option<String>>,
    /// The picture it shows.
    pub image: Signal<Option<Arc<ImageData>>>,
}
impl Player {
    /// A player of the script at `path` (a file, or a directory holding
    /// `video.luau`), its frames drawn `scale` pixels a point (no wider than
    /// 1280). It shows nothing until [`Player::open`].
    pub fn new(runtime: &Runtime, path: &Path, scale: f32) -> Result<Rc<Self>, String> {
        crate::script::entry_of(path)?;
        let (asks, asked) = channel();
        let (tell, told) = channel::<Event>();
        let (wake_up, woken) = zgx::task::latest::<()>();
        let path = path.to_owned();
        std::thread::Builder::new()
            .name("zgx-video player".into())
            .spawn(move || {
                run(
                    path,
                    scale,
                    asked,
                    tell,
                    Box::new(move || {
                        wake_up.send(());
                    }),
                );
            })
            .map_err(|error| error.to_string())?;
        Ok(Rc::new(Self {
            asks,
            events: RefCell::new(Some((told, woken))),
            frame: runtime.signal(0),
            playing: runtime.signal(false),
            composition: runtime.signal(None),
            tracks: runtime.signal(Vec::new()),
            error: runtime.signal(None),
            image: runtime.signal(None),
        }))
    }
    /// Show composition `id` with `props` over its defaults, at `frame`.
    pub fn open(&self, id: &str, props: Json, frame: u64) {
        let _ = self.asks.send(Ask::Open(id.to_owned(), props, frame));
    }
    /// Show `frame`.
    pub fn seek(&self, frame: u64) {
        let _ = self.asks.send(Ask::Seek(frame));
    }
    /// Play at real speed, or stop.
    pub fn play(&self, on: bool) {
        self.playing.set(on);
        let _ = self.asks.send(Ask::Play(on));
    }
    /// Whether what it shows is seen (true to begin with). Unseen, it draws
    /// nothing, playing or not: its clock runs on, and seen again it draws
    /// the frame due then. [`Player::view`] says so itself, from its box's
    /// visibility and its window's; a script's player is told by
    /// `"<name>.watch"`.
    pub fn watch(&self, seen: bool) {
        let _ = self.asks.send(Ask::Watch(seen));
    }
    /// Play again from its start at its end (true by default), or stop there.
    pub fn looping(&self, on: bool) {
        let _ = self.asks.send(Ask::Loop(on));
    }
    /// Build the script again (it changed), staying at the frame it shows.
    pub fn reload(&self) {
        let _ = self.asks.send(Ask::Reload);
    }
    /// Follow what its thread tells, keeping its signals current and telling
    /// `heard` too, for as long as the returned future runs: spawn it on the
    /// window's tasks. Only the first call follows; later ones end at once.
    pub fn listen<F: FnMut(&Event) + 'static>(
        self: &Rc<Self>,
        mut heard: F,
    ) -> impl std::future::Future<Output = ()> + 'static + use<F> {
        let events = self.events.borrow_mut().take();
        let this = self.clone();
        async move {
            let Some((told, mut woken)) = events else {
                return;
            };
            while woken.recv().await.is_some() {
                // The latest frame only: a picture each frame is enough to show.
                let mut frame = None;
                for event in told.try_iter() {
                    match &event {
                        Event::Frame { .. } => frame = Some(event),
                        other => {
                            this.keep(other);
                            heard(other);
                        }
                    }
                }
                if let Some(event) = frame {
                    this.keep(&event);
                    heard(&event);
                }
            }
        }
    }
    fn keep(&self, event: &Event) {
        match event {
            Event::Frame {
                frame,
                width,
                height,
                pixels,
            } => {
                if let Ok(image) = ImageData::new_premultiplied(*width, *height, pixels.clone()) {
                    self.image.set(Some(Arc::new(image)));
                }
                self.frame.set(*frame);
                self.error.set(None);
            }
            Event::Opened { composition } => {
                self.composition.set(Some((**composition).clone()));
                self.tracks.set(Vec::new());
            }
            Event::Tracks(tracks) => {
                self.tracks.set(tracks.clone());
            }
            Event::Stopped => {
                self.playing.set(false);
            }
            Event::Error(error) => {
                self.error.set(Some(error.clone()));
            }
        }
    }
    /// What it shows, as a view: its picture, scaled to fit the box it's given
    /// (give it a size), following it as it plays.
    pub fn view(self: &Rc<Self>) -> zgx::compose::View {
        let this = self.clone();
        zgx::compose::component(move |cx| {
            cx.tasks().spawn(this.listen(|_| {}));
            let image = this.image.clone();
            let blank = Arc::new(ImageData::new(1, 1, vec![0u8; 4]).expect("a pixel"));
            // Seen while any of its box shows, in a window that shows: else
            // its thread draws nothing (`Player::watch`).
            let watching: Rc<RefCell<Option<zgx::reactive::Effect>>> = Rc::default();
            cx.retain(watching.clone());
            // Gone, unseen.
            struct Unwatch(Rc<Player>);
            impl Drop for Unwatch {
                fn drop(&mut self) {
                    self.0.watch(false);
                }
            }
            cx.retain(Unwatch(this.clone()));
            let (player, runtime) = (this.clone(), cx.runtime());
            zgx::compose::image_signal("Video", move || {
                image.get().unwrap_or_else(|| blank.clone())
            })
            .on_mount(move |handle| {
                let (Some(visible), Some(shown)) =
                    (handle.observe_visibility(), handle.observe_shown())
                else {
                    return;
                };
                let mut told = None;
                *watching.borrow_mut() = Some(runtime.effect_now(move || {
                    let seen = visible.get() && shown.get();
                    if told != Some(seen) {
                        told = Some(seen);
                        player.watch(seen);
                    }
                }));
            })
        })
    }
}
impl Player {
    /// Give a script `host` runs this player as `name`: `ui.image { src =
    /// "host:<name>" }` shows it, `ui.call("<name>.listen")` (once, as the
    /// script mounts) follows it, and `ui.invoke("<name>.open", { id, props,
    /// frame })`, `"<name>.seek"` (a frame), `"<name>.play"` and `"<name>.loop"`
    /// (booleans) drive it. `ui.shared("<name>")` holds `{ frame, playing,
    /// composition, error }`.
    pub fn expose(self: &Rc<Self>, host: &zgx_script::ScriptHost, name: &str) {
        use zgx_script::Value;
        let state = host.shared(name, Value::from_json("{}").unwrap_or_default());
        let json_of =
            |value: &Value| serde_json::from_str::<Json>(&value.to_json()).unwrap_or(Json::Null);
        // What the script reads of it, told again as it changes: at each
        // event, and as it's played or stopped (which sends none: a stopped
        // player draws nothing more to tell it by).
        let tell: Rc<dyn Fn()> = Rc::new({
            let (player, state) = (self.clone(), state.clone());
            move || {
                let composition = player
                    .composition
                    .with_untracked(|c| c.as_ref().map(|c| c.id.clone()));
                let told = json!({
                    "frame": player.frame.with_untracked(|f| *f),
                    "playing": player.playing.with_untracked(|p| *p),
                    "composition": composition,
                    "error": player.error.with_untracked(Clone::clone),
                });
                state.set(Value::from_json(&told.to_string()).unwrap_or_default());
            }
        });
        let (this, shown, image) = (self.clone(), host.clone(), name.to_owned());
        let telling = tell.clone();
        host.async_action(format!("{name}.listen"), move |_| {
            let (shown, image, tell) = (shown.clone(), image.clone(), telling.clone());
            let listen = this.listen(move |event| {
                if let Event::Frame {
                    width,
                    height,
                    pixels,
                    ..
                } = event
                    && let Ok(picture) =
                        ImageData::new_premultiplied(*width, *height, pixels.clone())
                {
                    shown.image(&image, Some(Arc::new(picture)));
                }
                tell();
            });
            async move {
                listen.await;
                Ok(Value::Nil)
            }
        });
        let this = self.clone();
        host.action(format!("{name}.open"), move |arg| {
            let arg = json_of(&arg);
            this.open(
                arg["id"].as_str().unwrap_or_default(),
                arg.get("props").cloned().unwrap_or(json!({})),
                arg["frame"].as_f64().unwrap_or(0.).max(0.) as u64,
            );
        });
        let this = self.clone();
        host.action(format!("{name}.seek"), move |arg| {
            this.seek(json_of(&arg).as_f64().unwrap_or(0.).max(0.) as u64);
        });
        let this = self.clone();
        host.action(format!("{name}.play"), move |arg| {
            this.play(json_of(&arg).as_bool().unwrap_or(false));
            tell();
        });
        let this = self.clone();
        host.action(format!("{name}.loop"), move |arg| {
            this.looping(json_of(&arg).as_bool().unwrap_or(true))
        });
        let this = self.clone();
        host.action(format!("{name}.watch"), move |arg| {
            this.watch(json_of(&arg).as_bool().unwrap_or(true))
        });
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.asks.send(Ask::Quit);
    }
}
