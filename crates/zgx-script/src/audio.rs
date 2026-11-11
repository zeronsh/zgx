//! The Luau `audio` library (the `audio` feature), on zgx-audio: sounds
//! decoded off the main thread, music streamed, voices, buses, and `sound =`
//! on controls. Nothing here runs per audio block. A followed value (a
//! state, a function) is one command a change; a binding (`ui.tween`,
//! `ui.spring`) one a frame while it eases and none at rest; a bus's gain is
//! a store the mixer reads (`BusDrive`), so even a binding sends nothing.
//!
//! The engine is the host's (`ScriptHost::set_audio`), made the first time
//! a script calls `audio`: a script that never does makes none.
use crate::Vm;
use crate::node::{Anim, AnimUd, Curve};
use crate::timers::Timers;
use crate::value::{ReadState, State, Value};
use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::poll_fn;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant, SystemTime};
use zgx::frame::FrameClock;
use zgx::reactive::{Effect, Runtime, Signal};
use zgx_audio::{Audio, Play, Ramp, Sound, Source, Time, Voice};

/// How long a play asked before its sound is ready may wait for it: past
/// it, a click would come long after its button, so it's dropped.
const LATE: Duration = Duration::from_millis(250);
/// Voices a plugin plays at once.
const PLUGIN_VOICES: usize = 8;
/// A followed value's glide to each new value: long enough not to click.
const GLIDE: f64 = 0.010;
/// Decoded bytes of sounds no script holds that the app's cache keeps
/// (for the next reload to find) before it lets them go.
const UNHELD_BYTES: usize = 64 << 20;
/// Threads that decode sounds, and loads that may wait for one.
const LOADERS: usize = 2;
const LOADS_WAITING: usize = 256;

/// The scripts' side of the host's engine: what plays, what follows state,
/// and what each load started (stopped when a reload replaces it).
pub(crate) struct AudioHost {
    make: RefCell<Option<Rc<dyn Fn() -> Audio>>>,
    engine: RefCell<Option<Audio>>,
    runtime: Runtime,
    /// Where its tasks run: the host's `ui.every`'s.
    timers: RefCell<Option<Rc<Timers>>>,
    /// The window's display frames, once the host's view mounts: what a
    /// binding steps on.
    frames: Rc<RefCell<Option<FrameClock>>>,
    /// `audio.ui_sounds_enabled`: the user's interface-sound setting.
    ui_sounds: Signal<Value>,
    /// Plays dropped for waiting on their sound past `LATE`.
    late: Cell<u64>,
    /// Sounds asked for, counted: when each was last (`SoundSlot::used`).
    uses: Cell<u64>,
    /// The load that runs now, or ran last: what it starts is its.
    epoch: Cell<u64>,
    /// Voices playing, by id.
    voices: RefCell<HashMap<u64, Live>>,
    /// What follows state into a bus (its gain, mute or solo), by bus and
    /// what: the load that set it, and the follower.
    buses: RefCell<HashMap<BusParam, (u64, Follow)>>,
    /// The app's sounds by path, with the file's time when read: kept
    /// across reloads while a script holds them (`trim`). A plugin's are
    /// its own, counted against its memory, and go with it.
    sounds: RefCell<HashMap<PathBuf, Rc<SoundSlot>>>,
    /// Every sound made, the app's and plugins', for the plays waiting on
    /// them a reload stops.
    slots: RefCell<Vec<Weak<SoundSlot>>>,
    /// The threads that decode sounds: made with the first, parked while
    /// nothing loads.
    loader: RefCell<Option<zgx::background::BackgroundPool>>,
}

/// A bus's name and what of it is followed (`"gain"`, `"mute"`, `"solo"`).
type BusParam = (String, &'static str);

/// A voice that plays, or is due to.
struct Live {
    epoch: u64,
    voice: Voice,
    playing: Signal<Value>,
    /// Its gain's, pan's and pitch's followers.
    follows: [Option<Follow>; 3],
    /// The plugin that played it.
    plugin: Option<Rc<str>>,
}

impl AudioHost {
    pub(crate) fn new(runtime: &Runtime) -> Rc<Self> {
        Rc::new(AudioHost {
            make: RefCell::default(),
            engine: RefCell::default(),
            runtime: runtime.clone(),
            timers: RefCell::default(),
            frames: Rc::default(),
            ui_sounds: runtime.signal(Value::Bool(true)),
            late: Cell::new(0),
            uses: Cell::new(0),
            epoch: Cell::new(0),
            voices: RefCell::default(),
            buses: RefCell::default(),
            sounds: RefCell::default(),
            slots: RefCell::default(),
            loader: RefCell::default(),
        })
    }

    /// The engine scripts play through, made by `make` when a script first
    /// needs it.
    pub(crate) fn set_make(&self, make: Rc<dyn Fn() -> Audio>) {
        *self.make.borrow_mut() = Some(make);
    }

    /// The engine, if a script has made it.
    pub(crate) fn made(&self) -> Option<Audio> {
        self.engine.borrow().clone()
    }

    fn engine(&self) -> mlua::Result<Audio> {
        if let Some(audio) = &*self.engine.borrow() {
            return Ok(audio.clone());
        }
        let make = self.make.borrow().clone().ok_or_else(|| {
            mlua::Error::runtime(
                "audio: this host gives scripts no audio engine (ScriptHost::set_audio)",
            )
        })?;
        let audio = make();
        self.ui_sounds.set(Value::Bool(audio.ui_sounds_enabled()));
        *self.engine.borrow_mut() = Some(audio.clone());
        Ok(audio)
    }

    pub(crate) fn set_timers(&self, timers: Rc<Timers>) {
        *self.timers.borrow_mut() = Some(timers);
    }

    fn spawn(&self, task: impl std::future::Future<Output = ()> + 'static) {
        if let Some(timers) = &*self.timers.borrow() {
            timers.spawn(task);
        }
    }

    pub(crate) fn set_frames(&self, frames: FrameClock) {
        *self.frames.borrow_mut() = Some(frames);
    }

    pub(crate) fn set_ui_sounds(&self, enabled: bool) {
        if self.ui_sounds.with_untracked(Value::truthy) != enabled {
            self.ui_sounds.set(Value::Bool(enabled));
        }
    }

    /// Voices the engine says have ended: their `playing` turns false, and
    /// what followed state into them stops.
    pub(crate) fn voices_ended(&self, ids: &[u64]) {
        let ended: Vec<Live> = {
            let mut voices = self.voices.borrow_mut();
            ids.iter().filter_map(|id| voices.remove(id)).collect()
        };
        self.runtime.batch(|| {
            for live in &ended {
                live.playing.set(Value::Bool(false));
            }
        });
        drop(ended);
    }

    /// A load begins: what it starts is its own.
    pub(crate) fn begin_load(&self) {
        self.epoch.set(self.epoch.get() + 1);
    }

    /// The load that began last is done: what loads before it started stops
    /// (it replaced them), or, if it failed, what it started does (the
    /// scripts before it stay).
    pub(crate) fn end_load(&self, ok: bool) {
        let epoch = self.epoch.get();
        let replaced = |of: u64| if ok { of < epoch } else { of == epoch };
        let stopped: Vec<Live> = {
            let mut voices = self.voices.borrow_mut();
            let ids: Vec<u64> = voices
                .iter()
                .filter(|(_, live)| replaced(live.epoch))
                .map(|(id, _)| *id)
                .collect();
            ids.iter().filter_map(|id| voices.remove(id)).collect()
        };
        for live in &stopped {
            // A fade, never a cut.
            let _ = live.voice.stop();
        }
        self.runtime.batch(|| {
            for live in &stopped {
                live.playing.set(Value::Bool(false));
            }
        });
        drop(stopped);
        self.buses.borrow_mut().retain(|_, (of, _)| !replaced(*of));
        let slots: Vec<Rc<SoundSlot>> = {
            let mut slots = self.slots.borrow_mut();
            slots.retain(|slot| slot.strong_count() > 0);
            slots.iter().filter_map(Weak::upgrade).collect()
        };
        for slot in slots {
            slot.waiting.borrow_mut().retain(|waiting| {
                let keep = !replaced(waiting.epoch);
                if !keep {
                    waiting.cell.playing.set(Value::Bool(false));
                }
                keep
            });
        }
        if ok {
            self.trim(0);
        }
    }

    /// Lets go of the app's sounds no script holds, but for the most recent
    /// `keep` bytes of them: a reload that names one again finds it.
    pub(crate) fn trim(&self, keep: usize) {
        let mut sounds = self.sounds.borrow_mut();
        let mut unheld: Vec<(u64, usize, PathBuf)> = sounds
            .iter()
            // The cache's own reference alone: no script, node or load holds it.
            .filter(|(_, slot)| Rc::strong_count(slot) == 1)
            .map(|(path, slot)| (slot.used.get(), slot.bytes(), path.clone()))
            .collect();
        unheld.sort_unstable_by_key(|(used, ..)| std::cmp::Reverse(*used));
        let mut kept = 0;
        for (_, bytes, path) in unheld {
            kept += bytes;
            if kept > keep {
                sounds.remove(&path);
            }
        }
    }

    /// Decodes `path` on the loader's threads.
    fn decode(
        &self,
        audio: &Audio,
        path: PathBuf,
    ) -> Result<zgx::background::BackgroundTask<Result<Sound, String>>, String> {
        let mut loader = self.loader.borrow_mut();
        if loader.is_none() {
            *loader = Some(
                zgx::background::BackgroundPool::new(LOADERS, LOADS_WAITING)
                    .map_err(|error| format!("no thread to decode it on: {error}"))?,
            );
        }
        let audio = audio.clone();
        loader
            .as_ref()
            .expect("made above")
            .spawn(move || audio.load(Source::file(path)).map_err(|e| e.to_string()))
            .map_err(|error| format!("too many sounds loading at once: {error}"))
    }

    pub(crate) fn sounds_held(&self) -> usize {
        self.sounds.borrow().len()
    }

    fn plugin_voices(&self, plugin: &str) -> usize {
        self.voices
            .borrow()
            .values()
            .filter(|live| live.plugin.as_deref() == Some(plugin))
            .count()
    }

    /// A bus by name, a plugin's own made the first time it plays.
    fn plugin_bus(&self, audio: &Audio, plugin: &str) -> mlua::Result<zgx_audio::Bus> {
        let name = format!("plugin:{plugin}");
        if let Some(bus) = audio.bus(&name) {
            return Ok(bus);
        }
        let ui = audio.bus("ui").expect("the ui bus is made with the engine");
        audio.add_bus(&name, &ui).map_err(|e| fail("audio.play", e))
    }

    /// Starts `sound` as `spec` says, into `cell`.
    fn start(self: &Rc<Self>, sound: &Sound, cell: &Rc<VoiceCell>, spec: Spec) {
        let Ok(audio) = self.engine() else {
            cell.playing.set(Value::Bool(false));
            return;
        };
        let (gain, pan, pitch) = (spec.gain.now(), spec.pan.now(), spec.pitch.now());
        let mut play = Play::new()
            .bus(&spec.bus)
            .gain(gain)
            .pan(pan)
            .pitch(pitch)
            .priority(spec.priority);
        if let Some(at) = spec.at {
            play = play.at(at);
        }
        if spec.looping {
            play = match spec.loop_points {
                Some((start, end)) => {
                    let rate = f64::from(sound.rate());
                    let frame = |s: f64| (s.max(0.) * rate).round() as u64;
                    let end = end.map_or(sound.frames() as u64, frame);
                    play.loop_frames(frame(start)..end)
                }
                None => play.looping(true),
            };
        }
        if let Some(fade) = spec.fade_in {
            play = play.fade_in(fade);
        }
        match audio.play(sound, play) {
            Ok(voice) => self.track(voice, cell, spec, [gain, pan, pitch]),
            Err(error) => {
                eprintln!("audio.play: {error}");
                cell.playing.set(Value::Bool(false));
            }
        }
    }

    /// Keeps `voice`, now playing: its `playing` state, and what follows
    /// state into its gain, pan and pitch.
    fn track(self: &Rc<Self>, voice: Voice, cell: &Rc<VoiceCell>, spec: Spec, now: [f32; 3]) {
        *cell.voice.borrow_mut() = Some(voice.clone());
        let mut follows: [Option<Follow>; 3] = [None, None, None];
        for (at, param) in [spec.gain, spec.pan, spec.pitch].into_iter().enumerate() {
            if let Param::Followed(followed) = param {
                follows[at] = Some(self.follow_number(followed, now[at], voice_setter(&voice, at)));
            }
        }
        self.voices.borrow_mut().insert(
            voice.id(),
            Live {
                epoch: spec.epoch,
                voice,
                playing: cell.playing.clone(),
                follows,
                plugin: spec.plugin,
            },
        );
    }

    /// Follows a number from `followed`, which reads `now` at first, into
    /// `set`: each change once, with its glide (or a frame's, for a
    /// binding).
    fn follow_number(&self, followed: Followed, now: f32, set: Setter) -> Follow {
        match followed {
            Followed::Anim(anim) => self.follow_anim(anim, set),
            followed => {
                let last = Cell::new(now);
                self.follow(followed, move |value| {
                    let Some(value) = finite(value) else {
                        return;
                    };
                    if value != last.replace(value) {
                        set(value, GLIDE);
                    }
                })
            }
        }
    }

    /// Calls `on` with `followed`'s value now and on each change.
    fn follow(&self, followed: Followed, on: impl Fn(&Value) + 'static) -> Follow {
        let effect = match followed {
            Followed::State(state) => self.runtime.effect(move || state.with(|v| on(v))),
            Followed::Function(f, vm) => self.runtime.effect(move || {
                let Some(vm) = vm.upgrade() else { return };
                match vm.call::<Value>(&f, ()) {
                    Ok(value) => on(&value),
                    Err(error) => vm.report(error),
                }
            }),
            Followed::Anim(anim) => {
                // A boolean (mute, solo) from a binding: its source's.
                let Some(source) = anim.source.clone() else {
                    return Follow::none();
                };
                self.runtime.effect(move || source.with(|v| on(v)))
            }
        };
        Follow::none().with(effect)
    }

    /// Steps `anim` on the display's frames into `set` while it moves, as a
    /// bound property's animation does (`anim::drive`), with no component:
    /// a voice's or a bus's outlives what built it.
    fn follow_anim(&self, anim: Rc<Anim>, set: Setter) -> Follow {
        let follow = Follow::none();
        let (alive, wake) = (follow.alive.clone(), follow.wake.clone());
        // The display's frames, read as the task runs: one made before the
        // view mounted (at a script's top) runs once it has.
        let display = self.frames.clone();
        let Some(source) = anim.source.clone() else {
            let Curve::Loop {
                ms,
                ease,
                yoyo,
                fps,
            } = anim.curve.clone()
            else {
                return follow;
            };
            self.spawn(async move {
                let Some(frames) = display.borrow().clone() else {
                    return;
                };
                let start = frames.next().await.time;
                while alive.get() {
                    let frame = match fps {
                        Some(hz) => frames.next().max_rate(hz).await,
                        None => frames.next().await,
                    };
                    if !alive.get() {
                        return;
                    }
                    let cycles = (frame.time - start).as_secs_f32() * 1000. / ms;
                    let phase = if yoyo {
                        let p = cycles % 2.;
                        if p > 1. { 2. - p } else { p }
                    } else {
                        cycles.fract()
                    };
                    let value = ease.at(phase as f64, None) as f32;
                    set(map(&anim, value), frame.interval.as_secs_f64());
                }
            });
            return follow;
        };
        let target = Rc::new(Cell::new(source.with_untracked(number)));
        let effect = {
            let (target, wake) = (target.clone(), wake.clone());
            self.runtime.effect(move || {
                // Not finite, it keeps its goal: a NaN would never be
                // reached, and the binding would step every frame for good.
                let Some(next) = source.with(finite) else {
                    return;
                };
                if next != target.replace(next) {
                    wake.notify();
                }
            })
        };
        let curve = anim.curve.clone();
        let mut value = target.get();
        self.spawn(async move {
            loop {
                wake.wait().await;
                if !alive.get() {
                    return;
                }
                // No display (a headless host): each change one glide.
                let Some(frames) = display.borrow().clone() else {
                    value = target.get();
                    set(map(&anim, value), GLIDE);
                    continue;
                };
                let mut velocity = 0_f64;
                let mut start: Option<(Instant, f32, f32)> = None;
                let mut stepped: Option<Instant> = None;
                while value != target.get() {
                    let frame = frames.next().await;
                    if !alive.get() {
                        return;
                    }
                    value = match &curve {
                        Curve::Tween { ms, ease } => {
                            let (t0, from, to) = match start {
                                Some(s) if s.2 == target.get() => s,
                                _ => *start.insert((frame.time, value, target.get())),
                            };
                            let t = (frame.time - t0).as_secs_f32() * 1000. / *ms;
                            if t >= 1. {
                                to
                            } else {
                                let at = f64::from(from)
                                    + (f64::from(to) - f64::from(from)) * ease.at(t as f64, None);
                                at as f32
                            }
                        }
                        Curve::Spring { stiffness, damping } => {
                            let elapsed = stepped
                                .map_or(frame.interval, |last| {
                                    frame.time.saturating_duration_since(last)
                                })
                                .as_secs_f64()
                                .min(0.25);
                            stepped = Some(frame.time);
                            let goal = target.get();
                            let (mut x, to) = (f64::from(value), f64::from(goal));
                            let steps = (elapsed * 120.).ceil().max(1.);
                            let dt = elapsed / steps;
                            for _ in 0..steps as usize {
                                velocity += (-f64::from(*stiffness) * (x - to)
                                    - f64::from(*damping) * velocity)
                                    * dt;
                                x += velocity * dt;
                            }
                            let settle = 1e-3 * to.abs().max(1.);
                            let settled = !((x as f32).is_finite() && velocity.is_finite())
                                || (x - to).abs() < settle && velocity.abs() < settle * 10.;
                            if settled {
                                velocity = 0.;
                                goal
                            } else {
                                x as f32
                            }
                        }
                        Curve::Loop { .. } => target.get(),
                    };
                    set(map(&anim, value), frame.interval.as_secs_f64());
                }
            }
        });
        follow.with(effect)
    }
}

impl Drop for AudioHost {
    fn drop(&mut self) {
        // The host is going: what its scripts play stops, with its fade.
        for live in self.voices.get_mut().values() {
            let _ = live.voice.stop();
        }
    }
}

/// `with` a binding's progress mapped between its `from` and `to` (numbers).
fn map(anim: &Anim, value: f32) -> f32 {
    match (&anim.from, &anim.to) {
        (Some(from), Some(to)) => {
            let (from, to) = (number(from), number(to));
            from + (to - from) * value
        }
        _ => value,
    }
}

/// `value` as a number, 0 for none (and for one not finite).
fn number(value: &Value) -> f32 {
    finite(value).unwrap_or(0.)
}

/// `value` as a finite number, if it is one: a NaN or an infinity (a
/// script's `0 / 0`) is no value to move a sound to, and a follower keeps
/// the last one it had.
fn finite(value: &Value) -> Option<f32> {
    let n = match value {
        Value::Bool(b) => f32::from(u8::from(*b)),
        other => other.as_f64()? as f32,
    };
    n.is_finite().then_some(n)
}

/// Sets a number: the value, and over how many seconds.
type Setter = Rc<dyn Fn(f32, f64)>;

fn voice_setter(voice: &Voice, at: usize) -> Setter {
    let voice = voice.clone();
    Rc::new(move |value, seconds| {
        let ramp = Ramp::seconds(seconds);
        let sent = match at {
            0 => voice.set_gain(value, ramp),
            1 => voice.set_pan(value, ramp),
            _ => voice.set_pitch(value, ramp),
        };
        if let Err(error) = sent {
            eprintln!("audio: {error}");
        }
    })
}

/// A wake-up for one waiting task.
#[derive(Default)]
struct Notify {
    pending: Cell<bool>,
    waker: RefCell<Option<Waker>>,
}
impl Notify {
    fn notify(&self) {
        self.pending.set(true);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }
    async fn wait(&self) {
        poll_fn(|cx| {
            if self.pending.replace(false) {
                Poll::Ready(())
            } else {
                *self.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

/// What follows state into the engine: dropped, it stops (its effect, and
/// its task at the next frame or wake).
pub(crate) struct Follow {
    _effect: Option<Effect>,
    alive: Rc<Cell<bool>>,
    wake: Rc<Notify>,
}
impl Follow {
    fn with(mut self, effect: Effect) -> Follow {
        self._effect = Some(effect);
        self
    }
    fn none() -> Follow {
        Follow {
            _effect: None,
            alive: Rc::new(Cell::new(true)),
            wake: Rc::default(),
        }
    }
}
impl Drop for Follow {
    fn drop(&mut self) {
        self.alive.set(false);
        self.wake.notify();
    }
}

/// A value read once (a number) or followed.
#[derive(Clone)]
enum Param {
    Lit(f32),
    Followed(Followed),
}
#[derive(Clone)]
enum Followed {
    State(Signal<Value>),
    Function(mlua::Function, Weak<Vm>),
    Anim(Rc<Anim>),
}
impl Param {
    fn read(lua: &Lua, value: mlua::Value, default: f32, what: &str) -> mlua::Result<Param> {
        Ok(match Followed::read(lua, &value)? {
            Some(followed) => Param::Followed(followed),
            None => match value {
                mlua::Value::Nil => Param::Lit(default),
                mlua::Value::Integer(n) => Param::Lit(n as f32),
                mlua::Value::Number(n) => Param::Lit(n as f32),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "{what} takes a number, a state, a function or a binding, not {}",
                        other.type_name()
                    )));
                }
            },
        })
    }
    /// Its value now, read without following.
    fn now(&self) -> f32 {
        match self {
            Param::Lit(value) => *value,
            Param::Followed(followed) => followed.now(),
        }
    }
}
impl Followed {
    fn read(lua: &Lua, value: &mlua::Value) -> mlua::Result<Option<Followed>> {
        if let Some(state) = crate::node::state_of(value) {
            return Ok(Some(Followed::State(state)));
        }
        Ok(match value {
            mlua::Value::Function(f) => {
                let vm = crate::node::env(lua)?.vm.clone();
                Some(Followed::Function(f.clone(), vm))
            }
            mlua::Value::UserData(ud) if ud.is::<AnimUd>() => {
                Some(Followed::Anim(ud.borrow::<AnimUd>()?.0.clone()))
            }
            _ => None,
        })
    }
    fn now(&self) -> f32 {
        match self {
            Followed::State(state) => state.with_untracked(number),
            Followed::Function(f, vm) => vm.upgrade().map_or(0., |vm| {
                match vm.runtime.untracked(|| vm.call::<Value>(f, ())) {
                    Ok(value) => number(&value),
                    Err(error) => {
                        vm.report(error);
                        0.
                    }
                }
            }),
            Followed::Anim(anim) => map(
                anim,
                anim.source
                    .as_ref()
                    .map_or(0., |s| s.with_untracked(number)),
            ),
        }
    }
}

/// How a voice plays: `audio.play`'s options, read.
#[derive(Clone)]
struct Spec {
    bus: zgx_audio::Bus,
    gain: Param,
    pan: Param,
    pitch: Param,
    at: Option<Time>,
    looping: bool,
    /// `loop_start` and `loop_end`, in seconds.
    loop_points: Option<(f64, Option<f64>)>,
    priority: i8,
    fade_in: Option<Ramp>,
    /// The load it belongs to.
    epoch: u64,
    plugin: Option<Rc<str>>,
}

/// `audio.play`'s and `audio.music`'s options. `default_bus` names the bus
/// when they don't.
fn spec(
    lua: &Lua,
    host: &AudioHost,
    audio: &Audio,
    opts: Option<&Table>,
    default_bus: &str,
    plugin: Option<&Rc<str>>,
    call: &str,
) -> mlua::Result<Spec> {
    let get = |key: &str| -> mlua::Result<mlua::Value> {
        opts.map_or(Ok(mlua::Value::Nil), |t| t.get(key))
    };
    let bus = match (get("bus")?, plugin) {
        (mlua::Value::Nil, Some(plugin)) => host.plugin_bus(audio, plugin)?,
        (_, Some(_)) => {
            return Err(mlua::Error::runtime(format!(
                "{call}: a plugin plays into its own bus alone"
            )));
        }
        (mlua::Value::Nil, None) => audio
            .bus(default_bus)
            .expect("the engine makes its default buses"),
        (mlua::Value::String(name), None) => {
            let name = name.to_string_lossy();
            audio
                .bus(&name)
                .ok_or_else(|| mlua::Error::runtime(format!("{call}: no bus named {name:?}")))?
        }
        (mlua::Value::UserData(ud), None) if ud.is::<BusUd>() => ud.borrow::<BusUd>()?.bus.clone(),
        (other, None) => {
            return Err(mlua::Error::runtime(format!(
                "{call}: `bus` is a bus's name or a bus, not {}",
                other.type_name()
            )));
        }
    };
    let at = match get("at")? {
        mlua::Value::Nil => None,
        value => Some(time(audio, lua.unpack::<f64>(value)?)),
    };
    let loop_start = lua.unpack::<Option<f64>>(get("loop_start")?)?;
    let loop_end = lua.unpack::<Option<f64>>(get("loop_end")?)?;
    let loop_points =
        (loop_start.is_some() || loop_end.is_some()).then(|| (loop_start.unwrap_or(0.), loop_end));
    Ok(Spec {
        bus,
        gain: Param::read(lua, get("gain")?, 1., "`gain`")?,
        pan: Param::read(lua, get("pan")?, 0., "`pan`")?,
        pitch: Param::read(lua, get("pitch")?, 1., "`pitch`")?,
        at,
        looping: lua.unpack::<Option<bool>>(get("loop")?)?.unwrap_or(false)
            || loop_points.is_some(),
        loop_points,
        priority: lua
            .unpack::<Option<f64>>(get("priority")?)?
            .unwrap_or(0.)
            .clamp(-128., 127.) as i8,
        fade_in: match get("fade_in")? {
            mlua::Value::Nil => None,
            value => Some(fade(lua, value)?),
        },
        epoch: host.epoch.get(),
        plugin: plugin.cloned(),
    })
}

/// Seconds on the mixer's clock as its frame.
fn time(audio: &Audio, seconds: f64) -> Time {
    Time((seconds.max(0.) * f64::from(audio.rate())) as u64)
}

/// A fade: seconds, or `{ seconds, curve }`.
fn fade(lua: &Lua, value: mlua::Value) -> mlua::Result<Ramp> {
    match value {
        mlua::Value::Table(t) => {
            let seconds: f64 = t.get("seconds")?;
            let ramp = Ramp::seconds(seconds);
            Ok(match t.get::<Option<String>>("curve")? {
                Some(name) => ramp.curve(curve(&name)?),
                None => ramp,
            })
        }
        value => Ok(Ramp::seconds(lua.unpack::<f64>(value)?)),
    }
}

fn curve(name: &str) -> mlua::Result<zgx_audio::Curve> {
    Ok(match name {
        "linear" => zgx_audio::Curve::Linear,
        "equal_power" => zgx_audio::Curve::EqualPower,
        "exponential" => zgx_audio::Curve::Exponential,
        other => {
            return Err(mlua::Error::runtime(format!(
                "no curve {other:?}: \"linear\", \"equal_power\" or \"exponential\""
            )));
        }
    })
}

/// An engine error as `call`'s script error.
fn fail(call: &str, error: zgx_audio::Error) -> mlua::Error {
    mlua::Error::runtime(format!("{call}: {error}"))
}

/// A sound a script loaded (`audio.sound`): decoded on a thread of its own,
/// ready when it says so.
pub(crate) struct SoundSlot {
    host: Weak<AudioHost>,
    path: PathBuf,
    stamp: Option<SystemTime>,
    sound: RefCell<Option<Sound>>,
    ready: Signal<Value>,
    error: Signal<Value>,
    loading: Cell<bool>,
    /// Plays asked for before it was ready.
    waiting: RefCell<Vec<Waiting>>,
    /// A plugin's: what its decoded samples are counted against.
    meter: Option<Rc<crate::meter::Meter>>,
    /// When a script last asked for it, on the host's count: what `trim`
    /// keeps the most recent of.
    used: Cell<u64>,
}
struct Waiting {
    asked: Instant,
    cell: Rc<VoiceCell>,
    spec: Spec,
    epoch: u64,
}

impl SoundSlot {
    /// Decodes it, unless that's done or under way.
    fn load(self: &Rc<Self>, audio: &Audio) {
        if self.loading.replace(true) {
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let task = match host.decode(audio, self.path.clone()) {
            Ok(task) => task,
            Err(error) => {
                // Asked again later, it tries again.
                self.loading.set(false);
                self.loaded(Err(error));
                return;
            }
        };
        let slot = self.clone();
        host.spawn(async move {
            let loaded = match task.await {
                Ok(loaded) => loaded,
                Err(error) => Err(error.to_string()),
            };
            slot.loaded(loaded);
        });
    }

    /// What its samples hold, decoded.
    fn bytes(&self) -> usize {
        self.sound.borrow().as_ref().map_or(0, |s| {
            s.frames() * s.channels() * std::mem::size_of::<f32>()
        })
    }

    /// The decoder answered: plays that waited start, if they're not late.
    fn loaded(self: &Rc<Self>, loaded: Result<Sound, String>) {
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        let Some(host) = self.host.upgrade() else {
            return;
        };
        // A plugin's samples count against its memory, for as long as it
        // holds the sound.
        let loaded = match (loaded, &self.meter) {
            (Ok(sound), Some(meter)) => {
                let bytes = sound.frames() * sound.channels() * std::mem::size_of::<f32>();
                match meter.room(bytes) {
                    Ok(()) => {
                        let slot = Rc::downgrade(self);
                        meter.hold_native(move || slot.strong_count() > 0, bytes);
                        Ok(sound)
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            (loaded, _) => loaded,
        };
        match loaded {
            Ok(sound) => {
                *self.sound.borrow_mut() = Some(sound.clone());
                self.ready.set(Value::Bool(true));
                let now = zgx::clock::now();
                for waiting in waiting {
                    if waiting.cell.cancelled.get() {
                        continue;
                    }
                    if now.saturating_duration_since(waiting.asked) > LATE {
                        host.late.set(host.late.get() + 1);
                        waiting.cell.playing.set(Value::Bool(false));
                        continue;
                    }
                    host.start(&sound, &waiting.cell, waiting.spec);
                }
            }
            Err(error) => {
                eprintln!("audio.sound: {error}");
                self.error.set(Value::Str(error.into()));
                for waiting in waiting {
                    waiting.cell.playing.set(Value::Bool(false));
                }
            }
        }
    }

    /// Plays it as `spec` says: now if it's ready, when it is if it isn't.
    fn play(self: &Rc<Self>, host: &Rc<AudioHost>, audio: &Audio, spec: Spec) -> Rc<VoiceCell> {
        let cell = Rc::new(VoiceCell {
            voice: RefCell::default(),
            playing: host.runtime.signal(Value::Bool(true)),
            cancelled: Cell::new(false),
        });
        let sound = self.sound.borrow().clone();
        match sound {
            Some(sound) => host.start(&sound, &cell, spec),
            None if self.error.with_untracked(|e| !matches!(e, Value::Nil)) => {
                cell.playing.set(Value::Bool(false));
            }
            None => {
                self.waiting.borrow_mut().push(Waiting {
                    asked: zgx::clock::now(),
                    cell: cell.clone(),
                    epoch: spec.epoch,
                    spec,
                });
                self.load(audio);
            }
        }
        cell
    }
}

/// A voice as a script holds it: none until its sound is ready.
pub(crate) struct VoiceCell {
    voice: RefCell<Option<Voice>>,
    playing: Signal<Value>,
    /// Stopped before it started.
    cancelled: Cell<bool>,
}

/// `audio.sound`'s handle.
pub(crate) struct SoundUd(pub(crate) Rc<SoundSlot>);
impl UserData for SoundUd {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("ready", |_, s| {
            Ok(ReadState(State::new(s.0.ready.clone()), "ready".into()))
        });
        fields.add_field_method_get("error", |_, s| {
            Ok(ReadState(State::new(s.0.error.clone()), "error".into()))
        });
        fields.add_field_method_get("duration", |_, s| {
            Ok(s.0
                .sound
                .borrow()
                .as_ref()
                .map_or(0., |sound| sound.duration().as_secs_f64()))
        });
    }
}

/// `audio.play`'s and `audio.music`'s handle; `at` places what it's told
/// next on a frame.
pub(crate) struct VoiceUd {
    cell: Rc<VoiceCell>,
    host: Rc<AudioHost>,
    at: Option<Time>,
}
impl VoiceUd {
    /// The engine's voice, placed at `at`; none before it started.
    fn voice(&self) -> Option<Voice> {
        let voice = self.cell.voice.borrow().clone()?;
        Some(match self.at {
            Some(at) => voice.at(at),
            None => voice,
        })
    }
    fn set(&self, lua: &Lua, at: usize, value: mlua::Value, what: &str) -> mlua::Result<()> {
        let param = Param::read(lua, value, 0., what)?;
        let Some(voice) = self.voice() else {
            return Ok(());
        };
        let set = voice_setter(&voice, at);
        let follow = match param {
            Param::Lit(value) => {
                set(value, GLIDE);
                None
            }
            Param::Followed(followed) => {
                let now = followed.now();
                set(now, GLIDE);
                Some(self.host.follow_number(followed, now, set))
            }
        };
        if let Some(live) = self.host.voices.borrow_mut().get_mut(&voice.id()) {
            live.follows[at] = follow;
        }
        Ok(())
    }
}
impl UserData for VoiceUd {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("playing", |_, v| {
            Ok(ReadState(
                State::new(v.cell.playing.clone()),
                "playing".into(),
            ))
        });
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("stop", |_, v, seconds: Option<f64>| {
            match v.voice() {
                Some(voice) => match seconds {
                    Some(seconds) => voice.stop_after(Ramp::seconds(seconds)),
                    None => voice.stop(),
                }
                .map_err(|e| fail("voice:stop", e)),
                // Not started: it never will.
                None => {
                    v.cell.cancelled.set(true);
                    v.cell.playing.set(Value::Bool(false));
                    Ok(())
                }
            }
        });
        methods.add_method(
            "fade",
            |_, v, (gain, seconds, name): (f32, f64, Option<String>)| {
                let mut ramp = Ramp::seconds(seconds);
                if let Some(name) = name {
                    ramp = ramp.curve(curve(&name)?);
                }
                let Some(voice) = v.voice() else {
                    return Ok(());
                };
                // A fade takes the gain over from what followed state.
                if let Some(live) = v.host.voices.borrow_mut().get_mut(&voice.id()) {
                    live.follows[0] = None;
                }
                voice
                    .set_gain(gain, ramp)
                    .map_err(|e| fail("voice:fade", e))
            },
        );
        for (name, at) in [("gain", 0), ("pan", 1), ("pitch", 2)] {
            methods.add_method(name, move |lua, v, value: mlua::Value| {
                v.set(lua, at, value, &format!("voice:{name}"))
            });
        }
        methods.add_method("at", |_, v, seconds: f64| {
            let at = v.host.engine()?;
            Ok(VoiceUd {
                cell: v.cell.clone(),
                host: v.host.clone(),
                at: Some(time(&at, seconds)),
            })
        });
    }
}

/// `audio.bus`'s handle.
pub(crate) struct BusUd {
    host: Rc<AudioHost>,
    bus: zgx_audio::Bus,
    name: String,
}
impl BusUd {
    /// Follows `value` into the bus with `on`, replacing what followed it
    /// there before; a literal is applied once.
    fn follow(
        &self,
        lua: &Lua,
        what: &'static str,
        value: mlua::Value,
        on: impl Fn(&Value) + 'static,
    ) -> mlua::Result<()> {
        let key = (self.name.clone(), what);
        let follow = match Followed::read(lua, &value)? {
            Some(Followed::Anim(anim)) if what == "gain" => {
                let on = Rc::new(on);
                // From where its source is now: the binding moves it on.
                on(&Value::Num(f64::from(Followed::Anim(anim.clone()).now())));
                self.host.follow_anim(
                    anim,
                    Rc::new(move |value, _| on(&Value::Num(f64::from(value)))),
                )
            }
            Some(followed) => self.host.follow(followed, on),
            None => {
                self.host.buses.borrow_mut().remove(&key);
                on(&lua.unpack::<Value>(value)?);
                return Ok(());
            }
        };
        self.host
            .buses
            .borrow_mut()
            .insert(key, (self.host.epoch.get(), follow));
        Ok(())
    }
}
impl UserData for BusUd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("gain", |lua, b, value: mlua::Value| {
            if !matches!(
                value,
                mlua::Value::Number(_) | mlua::Value::Integer(_) | mlua::Value::Function(_)
            ) && Followed::read(lua, &value)?.is_none()
            {
                return Err(mlua::Error::runtime(format!(
                    "bus:gain takes a number, a state, a function or a binding, not {}",
                    value.type_name()
                )));
            }
            // A store the mixer reads each block: no command, whatever the
            // rate it changes at.
            let drive = b.bus.drive();
            b.follow(lua, "gain", value, move |value| {
                if let Some(gain) = finite(value) {
                    drive.set(gain);
                }
            })
        });
        methods.add_method(
            "fade",
            |_, b, (gain, seconds, name): (f32, f64, Option<String>)| {
                let curve = match name {
                    Some(name) => curve(&name)?,
                    None => zgx_audio::Curve::Linear,
                };
                // A fade takes the gain over from what followed state.
                b.host.buses.borrow_mut().remove(&(b.name.clone(), "gain"));
                b.bus
                    .fade_to(gain, seconds, curve)
                    .map_err(|e| fail("bus:fade", e))
            },
        );
        for what in ["mute", "solo"] {
            methods.add_method(what, move |lua, b, value: mlua::Value| {
                let (bus, last) = (b.bus.clone(), Cell::new(None));
                b.follow(lua, what, value, move |value| {
                    let on = value.truthy();
                    if last.replace(Some(on)) == Some(on) {
                        return;
                    }
                    let sent = match what {
                        "mute" => bus.mute(on),
                        _ => bus.solo(on),
                    };
                    if let Err(error) = sent {
                        eprintln!("bus:{what}: {error}");
                    }
                })
            });
        }
        methods.add_method("at", |_, b, seconds: f64| {
            let audio = b.host.engine()?;
            Ok(BusUd {
                host: b.host.clone(),
                bus: b.bus.at(time(&audio, seconds)),
                name: b.name.clone(),
            })
        });
    }
}

/// Makes the `audio` global: a plugin's (`plugin`, its name) plays into a
/// bus of its own under `ui`, at most `PLUGIN_VOICES` at once, and reaches
/// no other bus, the clock's `prewarm`, or the system setting.
pub(crate) fn install(
    lua: &Lua,
    globals: &Table,
    host: Rc<AudioHost>,
    plugin: Option<Rc<str>>,
) -> mlua::Result<()> {
    let audio = lua.create_table()?;
    {
        let (host, plugin) = (host.clone(), plugin.clone());
        audio.set(
            "sound",
            lua.create_function(move |lua, (src, opts): (String, Option<Table>)| {
                let engine = host.engine()?;
                let path = resolve(lua, &src, plugin.is_some(), "audio.sound")?;
                let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                // The app's are shared and kept; a plugin's are its own.
                let cached = match plugin {
                    Some(_) => None,
                    None => host
                        .sounds
                        .borrow()
                        .get(&path)
                        .filter(|slot| slot.stamp == stamp)
                        .cloned(),
                };
                host.uses.set(host.uses.get() + 1);
                let slot = match cached {
                    Some(slot) => slot,
                    None => {
                        let meter = match plugin {
                            Some(_) => crate::node::env(lua)?.meter.clone(),
                            None => None,
                        };
                        let slot = Rc::new(SoundSlot {
                            host: Rc::downgrade(&host),
                            path: path.clone(),
                            stamp,
                            sound: RefCell::default(),
                            ready: host.runtime.signal(Value::Bool(false)),
                            error: host.runtime.signal(Value::Nil),
                            loading: Cell::new(false),
                            waiting: RefCell::default(),
                            meter,
                            used: Cell::new(0),
                        });
                        host.slots.borrow_mut().push(Rc::downgrade(&slot));
                        if plugin.is_none() {
                            host.sounds.borrow_mut().insert(path, slot.clone());
                            host.trim(UNHELD_BYTES);
                        }
                        slot
                    }
                };
                slot.used.set(host.uses.get());
                let preload = match &opts {
                    Some(opts) => opts.get::<Option<bool>>("preload")?.unwrap_or(true),
                    None => true,
                };
                if preload {
                    slot.load(&engine);
                }
                Ok(SoundUd(slot))
            })?,
        )?;
    }
    {
        let (host, plugin) = (host.clone(), plugin.clone());
        audio.set(
            "play",
            lua.create_function(
                move |lua, (sound, opts): (mlua::AnyUserData, Option<Table>)| {
                    let slot = sound
                        .borrow::<SoundUd>()
                        .map_err(|_| {
                            mlua::Error::runtime("audio.play takes a sound (audio.sound's)")
                        })?
                        .0
                        .clone();
                    let audio = host.engine()?;
                    let spec = spec(
                        lua,
                        &host,
                        &audio,
                        opts.as_ref(),
                        "world",
                        plugin.as_ref(),
                        "audio.play",
                    )?;
                    if let Some(plugin) = &plugin
                        && host.plugin_voices(plugin) >= PLUGIN_VOICES
                    {
                        // Refused: a voice that never plays.
                        let cell = Rc::new(VoiceCell {
                            voice: RefCell::default(),
                            playing: host.runtime.signal(Value::Bool(false)),
                            cancelled: Cell::new(true),
                        });
                        return Ok(VoiceUd {
                            cell,
                            host: host.clone(),
                            at: None,
                        });
                    }
                    let cell = slot.play(&host, &audio, spec);
                    Ok(VoiceUd {
                        cell,
                        host: host.clone(),
                        at: None,
                    })
                },
            )?,
        )?;
    }
    {
        let (host, plugin) = (host.clone(), plugin.clone());
        audio.set(
            "music",
            lua.create_function(move |lua, (src, opts): (String, Option<Table>)| {
                let audio = host.engine()?;
                let path = resolve(lua, &src, plugin.is_some(), "audio.music")?;
                // A stream is a voice too (and a window, a ring and decoding):
                // a plugin's count among its voices.
                if let Some(plugin) = &plugin
                    && host.plugin_voices(plugin) >= PLUGIN_VOICES
                {
                    return Ok(VoiceUd {
                        cell: Rc::new(VoiceCell {
                            voice: RefCell::default(),
                            playing: host.runtime.signal(Value::Bool(false)),
                            cancelled: Cell::new(true),
                        }),
                        host: host.clone(),
                        at: None,
                    });
                }
                let spec = spec(
                    lua,
                    &host,
                    &audio,
                    opts.as_ref(),
                    "music",
                    plugin.as_ref(),
                    "audio.music",
                )?;
                let (gain, pan, pitch) = (spec.gain.now(), spec.pan.now(), spec.pitch.now());
                let mut play = Play::new()
                    .bus(&spec.bus)
                    .gain(gain)
                    .pan(pan)
                    .pitch(pitch)
                    .priority(spec.priority)
                    .looping(spec.looping);
                if let Some(at) = spec.at {
                    play = play.at(at);
                }
                if let Some(fade) = spec.fade_in {
                    play = play.fade_in(fade);
                }
                let cell = Rc::new(VoiceCell {
                    voice: RefCell::default(),
                    playing: host.runtime.signal(Value::Bool(true)),
                    cancelled: Cell::new(false),
                });
                let voice = audio
                    .stream(Source::file(path), play)
                    .map_err(|e| fail("audio.music", e))?;
                host.track(voice, &cell, spec, [gain, pan, pitch]);
                Ok(VoiceUd {
                    cell,
                    host: host.clone(),
                    at: None,
                })
            })?,
        )?;
    }
    {
        let host = host.clone();
        audio.set(
            "now",
            lua.create_function(move |_, ()| {
                let audio = host.engine()?;
                Ok(audio.now().0 as f64 / f64::from(audio.rate()))
            })?,
        )?;
    }
    {
        let host = host.clone();
        audio.set(
            "stats",
            lua.create_function(move |lua, ()| {
                let stats = host.engine()?.stats();
                let t = lua.create_table()?;
                t.set("voices", stats.voices)?;
                t.set("stolen", stats.stolen)?;
                t.set("dropped", stats.dropped)?;
                t.set("late", stats.late + host.late.get())?;
                t.set("dry", stats.dry)?;
                t.set("streams", stats.streams)?;
                t.set("underruns", stats.underruns)?;
                t.set("running", stats.running)?;
                t.set("starts", stats.starts)?;
                t.set("stops", stats.stops)?;
                t.set("peak", stats.peak)?;
                Ok(t)
            })?,
        )?;
    }
    if plugin.is_none() {
        {
            let host = host.clone();
            audio.set(
                "bus",
                lua.create_function(move |_, name: String| {
                    let audio = host.engine()?;
                    Ok(audio.bus(&name).map(|bus| BusUd {
                        host: host.clone(),
                        bus,
                        name,
                    }))
                })?,
            )?;
        }
        {
            let host = host.clone();
            audio.set(
                "add_bus",
                lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
                    let audio = host.engine()?;
                    let parent = match &opts {
                        Some(opts) => opts.get::<Option<String>>("parent")?,
                        None => None,
                    }
                    .unwrap_or_else(|| "master".into());
                    let parent = audio.bus(&parent).ok_or_else(|| {
                        mlua::Error::runtime(format!("audio.add_bus: no bus named {parent:?}"))
                    })?;
                    // A reload makes it again: the bus there is the same one.
                    let bus = match audio.bus(&name) {
                        Some(bus) => bus,
                        None => audio
                            .add_bus(&name, &parent)
                            .map_err(|e| fail("audio.add_bus", e))?,
                    };
                    Ok(BusUd {
                        host: host.clone(),
                        bus,
                        name,
                    })
                })?,
            )?;
        }
        {
            let host = host.clone();
            audio.set(
                "prewarm",
                lua.create_function(move |_, ()| {
                    host.engine()?.prewarm();
                    Ok(())
                })?,
            )?;
        }
        audio.set(
            "ui_sounds_enabled",
            ReadState(
                State::new(host.ui_sounds.clone()),
                "ui_sounds_enabled".into(),
            ),
        )?;
    }
    audio.set_readonly(true);
    globals.raw_set("audio", audio)
}

/// `src` as a path: relative to the script, as images are. A plugin's
/// stays in its directory, as its images and modules do (`within`: by name
/// and by where a link leads), or a file its host's grant let it pick; a
/// refusal says nothing of what is there.
fn resolve(lua: &Lua, src: &str, plugin: bool, call: &str) -> mlua::Result<PathBuf> {
    let env = crate::node::env(lua)?;
    let base = env.vm.upgrade().and_then(|vm| vm.base.clone());
    let path = PathBuf::from(src);
    if !plugin {
        return Ok(match base {
            Some(base) if !path.is_absolute() => base.join(path),
            _ => path,
        });
    }
    if path.is_absolute() && env.picked.borrow().contains(&path) {
        return Ok(path);
    }
    let root = base.map(|base| base.canonicalize().unwrap_or(base));
    match root {
        Some(root) if crate::within(&root, &crate::normalize(&root.join(&path))) => {
            Ok(crate::normalize(&root.join(path)))
        }
        _ => Err(mlua::Error::runtime(format!(
            "{call}: a plugin's sounds come from its directory or below"
        ))),
    }
}

/// `sound =` on a control: what plays as it's activated.
#[derive(Clone)]
pub(crate) enum SoundProp {
    Sound {
        slot: Rc<SoundSlot>,
        gain: f32,
        pitch: f32,
    },
    /// A function answering one of the others, called at activation.
    Chosen(mlua::Function),
}

/// `sound =`'s value: a sound, `{ sound = …, gain, pitch }`, a function
/// answering either, or `false`/nil for none.
pub(crate) fn sound_prop(value: &mlua::Value) -> mlua::Result<Option<SoundProp>> {
    Ok(match value {
        mlua::Value::Nil | mlua::Value::Boolean(false) => None,
        mlua::Value::UserData(ud) if ud.is::<SoundUd>() => Some(SoundProp::Sound {
            slot: ud.borrow::<SoundUd>()?.0.clone(),
            gain: 1.,
            pitch: 1.,
        }),
        mlua::Value::Table(t) => {
            // `{ sound = click, gain = … }`, or the sound first.
            let sound = match t.get::<mlua::Value>("sound")? {
                mlua::Value::Nil => t.get::<mlua::Value>(1)?,
                sound => sound,
            };
            let mlua::Value::UserData(ud) = sound else {
                return Err(mlua::Error::runtime(
                    "`sound` takes { sound = …, gain = …, pitch = … }",
                ));
            };
            let slot = ud
                .borrow::<SoundUd>()
                .map_err(|_| mlua::Error::runtime("`sound`'s sound is audio.sound's"))?
                .0
                .clone();
            Some(SoundProp::Sound {
                slot,
                gain: t.get::<Option<f32>>("gain")?.unwrap_or(1.),
                pitch: t.get::<Option<f32>>("pitch")?.unwrap_or(1.),
            })
        }
        mlua::Value::Function(f) => Some(SoundProp::Chosen(f.clone())),
        other => {
            return Err(mlua::Error::runtime(format!(
                "`sound` takes audio.sound's sound, {{ sound = …, gain, pitch }}, a function \
                 or false, not {}",
                other.type_name()
            )));
        }
    })
}

/// A control with `sound` was activated: its sound plays on `ui` now,
/// before the handler runs.
pub(crate) fn activate(prop: &SoundProp, vm: &Vm) {
    let prop = match prop {
        SoundProp::Chosen(f) => {
            let chosen = vm
                .runtime
                .untracked(|| vm.call::<mlua::Value>(f, ()))
                .and_then(|value| sound_prop(&value));
            match chosen {
                Ok(Some(prop @ SoundProp::Sound { .. })) => prop,
                Ok(_) => return,
                Err(error) => {
                    vm.report(error);
                    return;
                }
            }
        }
        prop => prop.clone(),
    };
    let SoundProp::Sound { slot, gain, pitch } = prop else {
        return;
    };
    let Some(host) = slot.host.upgrade() else {
        return;
    };
    let Ok(audio) = host.engine() else {
        return;
    };
    let ui = audio.bus("ui").expect("the ui bus is made with the engine");
    let spec = Spec {
        bus: ui,
        gain: Param::Lit(gain),
        pan: Param::Lit(0.),
        pitch: Param::Lit(pitch),
        at: None,
        looping: false,
        loop_points: None,
        priority: 0,
        fade_in: None,
        epoch: host.epoch.get(),
        plugin: None,
    };
    slot.play(&host, &audio, spec);
}
