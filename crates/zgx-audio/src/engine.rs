//! The main thread's side: [`Audio`] and the handles it gives out. Each
//! call becomes a command stamped with a frame and goes on a fixed queue;
//! a full queue is an error, never a wait.
use crate::device::{Device, Host, Message, Renderer};
use crate::effect::{Effect, Slot};
use crate::mixer::Streamed;
use crate::mixer::{
    BusMemory, Command, FADING_SLOTS, Feed, MAX_BUSES, MAX_EFFECTS, Meter, Mixer, PanLaw, Return,
    Setup, VoiceParam,
};
use crate::queue::{Consumer, Producer, queue};
use crate::ramp::{Curve, Ramp};
use crate::resample::{Interpolation, Region};
use crate::ring::ring;
use crate::sound::{Sound, Source};
use crate::stream::{Decoder, Feeding};
use std::collections::HashSet;
use std::ops::{Add, Range, Sub};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The command queue is full: the mixer hasn't run for a while, or
    /// more was sent in one go than [`Config::commands`] holds (of which a
    /// quarter is kept for commands due now: ones for later get this
    /// sooner).
    QueueFull,
    /// [`Audio::add_bus`] past the most buses there can be (64).
    TooManyBuses,
    /// [`Bus::add_effect`] past the most effects a bus can hold (8).
    TooManyEffects,
    /// A bus of this name is there already.
    BusExists(String),
    /// Bytes that aren't a sound zgx-audio reads, and why.
    Format(String),
    /// A file that couldn't be read.
    Io(String),
    /// The output device couldn't be started, and why.
    Device(String),
    /// A [`Config`] the engine can't be made with, and why.
    Config(String),
    /// A handle (a [`Bus`]) from another engine.
    Foreign,
    /// An effect whose parameters can't be kept to (a range upside down, or
    /// not a number), and why.
    Effect(String),
    /// A stream past [`Config::stream_memory`]: the bytes it would take,
    /// and those free.
    StreamMemory { asked: usize, free: usize },
}

impl Error {
    pub(crate) fn within(self, name: &str) -> Error {
        match self {
            Error::Format(why) => Error::Format(format!("{name}: {why}")),
            Error::Io(why) => Error::Io(format!("{name}: {why}")),
            other => other,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::QueueFull => f.write_str("the audio command queue is full"),
            Error::TooManyBuses => write!(f, "there can be at most {MAX_BUSES} buses"),
            Error::TooManyEffects => {
                write!(f, "a bus can hold at most {MAX_EFFECTS} effects")
            }
            Error::BusExists(name) => write!(f, "a bus named {name:?} exists already"),
            Error::Format(why) | Error::Io(why) => f.write_str(why),
            Error::Device(why) => write!(f, "the audio device: {why}"),
            Error::Config(why) => write!(f, "an audio engine can't be made with {why}"),
            Error::Foreign => f.write_str("a bus from another audio engine"),
            Error::Effect(why) => write!(f, "an effect with {why}"),
            Error::StreamMemory { asked, free } => write!(
                f,
                "a stream needing {asked} bytes, with {free} of the streams' memory free"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A frame on the mixer's clock, counted from its first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(pub u64);

impl Add<u64> for Time {
    type Output = Time;
    /// Past the clock's last frame is its last frame: never.
    fn add(self, frames: u64) -> Time {
        Time(self.0.saturating_add(frames))
    }
}

impl Sub<Time> for Time {
    type Output = u64;
    fn sub(self, earlier: Time) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// How an engine is made.
#[derive(Clone, Debug)]
pub struct Config {
    /// Frames a second.
    pub rate: u32,
    /// Output channels: stereo is mixed, and copied or summed to these.
    pub channels: usize,
    /// Voices that can play at once; past it, the quietest of the lowest
    /// priority is stolen with a 5 ms fade.
    pub voices: usize,
    /// Commands that can wait for the mixer at once.
    pub commands: usize,
    /// The ceiling of master's look-ahead limiter (1.5 ms), or `None` for
    /// no limiter.
    pub limiter: Option<f32>,
    /// How voices read between frames when a sound's rate or pitch isn't
    /// the output's.
    pub interpolation: Interpolation,
    /// Master's gain at the start, before any command.
    pub gain: f32,
    /// How long a device keeps running after the last sound (fades, the
    /// limiter's tail and sounds due count as sound): then it stops, and
    /// starts again with the next.
    pub hold: Duration,
    /// How long a stopped stream may stay paused
    /// ([`Stopping::Pause`](crate::Stopping::Pause)) before it's closed
    /// (10 s): a paused stream still wakes the process on some systems.
    pub close_after: Duration,
    /// Bytes all streams' buffers may take at once (8 MiB): each holds
    /// half a second in its ring and 32,768 frames to read from, 448 KiB
    /// for a stereo stream at 48 kHz. Past it, [`Audio::stream`] says
    /// [`Error::StreamMemory`]; a stream's bytes come back when the mixer
    /// gives it back.
    pub stream_memory: usize,
}

/// Ended voices kept for [`Audio::take_ended`] at most.
const MAX_ENDED: usize = 1 << 16;

/// The most voices an engine can have.
pub const MAX_VOICES: usize = 4096;

/// The most a voice's or a bus's gain can be: +18 dB. With samples kept
/// to ±[`MAX_SAMPLE`](crate::MAX_SAMPLE), the loudest a sum can get through
/// the deepest tree of buses is about 2^115, so every sample the mixer
/// sends is a finite number, with no check on the way.
pub const MAX_GAIN: f32 = 8.;

/// Values from the app, made what the mixer can rely on: here, on the main
/// thread, so the audio thread checks nothing. A gain is kept from 0 to
/// [`MAX_GAIN`] (not a number: 0), a pan from -1 to 1 (not a number: 0), a
/// pitch from 1/64 to 16 (not a number: 1).
fn gain(value: f32) -> f32 {
    if value.is_nan() {
        0.
    } else {
        value.clamp(0., MAX_GAIN)
    }
}
fn pan(value: f32) -> f32 {
    if value.is_nan() {
        0.
    } else {
        value.clamp(-1., 1.)
    }
}
fn pitch(value: f32) -> f32 {
    if value.is_nan() {
        1.
    } else {
        value.clamp(1. / 64., 16.)
    }
}
/// A voice's low-pass cutoff, from 10 Hz to open (not a number: open).
fn cutoff(value: f32) -> f32 {
    if value.is_nan() {
        crate::effects::VOICE_OPEN
    } else {
        value.clamp(10., crate::effects::VOICE_OPEN)
    }
}

impl Config {
    /// Whether an engine can be made with it: a rate from 1 kHz to
    /// 768 kHz, 1 to 64 channels, 1 to [`MAX_VOICES`] voices, 16 to 2^20
    /// commands, a limiter's ceiling finite and above zero, and a gain from
    /// 0 to [`MAX_GAIN`].
    pub fn check(&self) -> Result<(), Error> {
        let fail = |why: String| Err(Error::Config(why));
        if !(1_000..=768_000).contains(&self.rate) {
            return fail(format!("a rate of {} Hz (1 kHz to 768 kHz)", self.rate));
        }
        if !(1..=64).contains(&self.channels) {
            return fail(format!("{} channels (1 to 64)", self.channels));
        }
        if !(1..=MAX_VOICES).contains(&self.voices) {
            return fail(format!("{} voices (1 to {MAX_VOICES})", self.voices));
        }
        if !(16..=1 << 20).contains(&self.commands) {
            return fail(format!("room for {} commands (16 to 2^20)", self.commands));
        }
        if let Some(ceiling) = self.limiter
            && !(ceiling.is_finite() && ceiling > 0.)
        {
            return fail(format!("a limiter ceiling of {ceiling} (above 0, finite)"));
        }
        if !(0. ..=MAX_GAIN).contains(&self.gain) {
            return fail(format!("a gain of {} (0 to {MAX_GAIN})", self.gain));
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            rate: 48_000,
            channels: 2,
            voices: 64,
            commands: 1024,
            limiter: Some(1.),
            interpolation: Interpolation::Cubic,
            gain: 1.,
            hold: Duration::from_secs(1),
            close_after: Duration::from_secs(10),
            stream_memory: 8 << 20,
        }
    }
}

/// What the mixer has done, as of its last block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// Frames mixed.
    pub frames: u64,
    /// Blocks asked for.
    pub blocks: u64,
    /// Voices playing, fading or waiting to be given back.
    pub voices: u64,
    /// Voices stolen for a new one.
    pub stolen: u64,
    /// Plays that lost to every voice playing and never started.
    pub dropped: u64,
    /// Commands applied after the frame they asked for.
    pub late: u64,
    /// Commands sent to the mixer since the engine was made: what a
    /// caller counts to show a change made none (a bus's drive).
    pub commands: u64,
    /// Frames mixed with sound reaching master, before its gain: what a run
    /// at zero gain mixed.
    pub sounding: u64,
    /// Whether a device stream runs now.
    pub running: bool,
    /// Device streams started (or resumed) and stopped.
    pub starts: u64,
    pub stops: u64,
    /// How long the last start took the device, in microseconds.
    pub start_micros: u64,
    /// Streams lost (a device gone, a format changed) and started again.
    pub losses: u64,
    /// Starts that failed: see [`Audio::device_error`].
    pub failures: u64,
    /// Underruns the system reported.
    pub underruns: u64,
    /// Times a stream ran dry: its decoding fell behind.
    pub dry: u64,
    /// Streams the decode thread is filling.
    pub streams: u64,
    /// Bytes streams' buffers hold ([`Config::stream_memory`]).
    pub stream_memory: u64,
    /// On the device's clock, in nanoseconds (0 until a device says): when
    /// the last block was asked for, when its first frame will be heard,
    /// and when the most recent play's first frame will be.
    pub block_asked: u64,
    pub block_heard: u64,
    pub play_heard: u64,
    /// The device's clock when the block that started the most recent play
    /// was asked for.
    pub play_asked: u64,
    /// The host's clock (macOS; else 0) when that block was mixed.
    pub play_mixed: u64,
    /// The loudest sample sent to the output since the engine was made.
    pub peak: f32,
    /// Times a play found every slot taken (by voices fading out after a
    /// steal) and waited a little to try again.
    pub waited: u64,
}

struct Control {
    commands: Producer<(u64, Command)>,
    /// The most commands that may wait for the mixer at once: its
    /// schedule's room.
    room: u64,
    returns: Consumer<Return>,
    next_id: u64,
    /// Voices given back since [`Audio::take_ended`] last took them.
    ended: Vec<u64>,
    /// Whether the `ui` bus plays: the user's interface-sound setting.
    ui_sounds: bool,
    /// Name and parent of each bus, by index.
    buses: Vec<(String, u16)>,
    /// Voices sent and not yet given back.
    live: HashSet<u64>,
    /// Each effect on a bus or on its way to one: its id, its bus, and
    /// whether its removal is sent for a later frame. One leaving holds
    /// its place until the mixer gives it back, so a bus never takes more
    /// than it can hold.
    effects: Vec<(u64, u16, bool)>,
}

/// The device and its host thread, made when first needed.
enum Hosting {
    /// No device: an offline engine.
    None,
    /// A device not started yet (or closed, its thread ended), the
    /// renderer it will run, and its hold and grace when paused.
    Waiting(Box<dyn Device>, Box<Renderer>, (Duration, Duration)),
    Running(Host),
}

/// Engines made so far: each takes the next as its identity, which its
/// handles carry.
static ENGINES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Inner {
    /// Which engine this is: a handle from another is refused.
    id: u64,
    control: Mutex<Control>,
    meter: Arc<Meter>,
    latency: usize,
    /// Shared with the host thread, which puts the device back here when
    /// it ends.
    hosting: Arc<Mutex<Hosting>>,
    /// The decode thread, made with the first stream.
    decoder: Mutex<Option<Decoder>>,
    /// The bytes streams hold now, and the most they may.
    streamed: Arc<std::sync::atomic::AtomicUsize>,
    stream_memory: usize,
}

/// The engine, as the main thread holds it. Cloning it is cheap; every
/// clone drives the same mixer.
#[derive(Clone)]
pub struct Audio(Arc<Inner>);

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("rate", &self.rate())
            .field("now", &self.now())
            .finish()
    }
}

/// The mixer run by hand, on a clock of its own: tests compare what it
/// makes sample for sample, and a video renders its soundtrack with it.
pub struct Offline {
    renderer: Renderer,
    channels: usize,
    audio: Audio,
}

impl Offline {
    /// Mixes the next `out.len() / channels` frames into `out`.
    /// Streams are filled first, so what comes out doesn't depend on how
    /// fast the decode thread ran.
    pub fn render(&mut self, out: &mut [f32]) {
        self.audio.settle();
        self.renderer.render(out, None);
    }

    /// Mixes without waiting for streams first, as a device does: a stream
    /// the decode thread hasn't filled waits, or runs dry.
    pub fn render_now(&mut self, out: &mut [f32]) {
        self.renderer.render(out, None);
    }

    /// The next `frames` frames, interleaved.
    pub fn frames(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.; frames * self.channels];
        self.render(&mut out);
        out
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
}

impl Audio {
    /// An engine whose mixer the caller runs, with [`Offline::render`].
    /// Fails for a config [`Config::check`] refuses.
    pub fn offline(config: Config) -> Result<(Audio, Offline), Error> {
        let (audio, mixer) = Audio::with_mixer(&config)?;
        let channels = config.channels;
        let renderer = Renderer::new(mixer, channels, audio.0.meter.clone());
        Ok((
            audio.clone(),
            Offline {
                renderer,
                channels,
                audio,
            },
        ))
    }

    /// An engine on the system's default output, through cpal. Nothing is
    /// opened until the first sound: then a stream starts, and it stops
    /// after [`Config::hold`] with nothing to hear. Fails for a config
    /// [`Config::check`] refuses.
    #[cfg(feature = "device")]
    pub fn new(config: Config) -> Result<Audio, Error> {
        Audio::with_device(config, crate::Cpal::new())
    }

    /// An engine on `device`, started with the first sound. Fails for a
    /// config [`Config::check`] refuses.
    pub fn with_device(config: Config, device: impl Device) -> Result<Audio, Error> {
        let channels = config.channels;
        let times = (config.hold, config.close_after);
        let (audio, mixer) = Audio::with_mixer(&config)?;
        let renderer = Box::new(Renderer::new(mixer, channels, audio.0.meter.clone()));
        *audio.hosting() = Hosting::Waiting(Box::new(device), renderer, times);
        Ok(audio)
    }

    fn hosting(&self) -> MutexGuard<'_, Hosting> {
        self.0
            .hosting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Tells the host a command waits, starting it the first time.
    fn wake(&self, message: Message) {
        let mut hosting = self.hosting();
        // Before any sound, a command that makes none (a gain, a mute, a
        // bus) is applied here: no device, no thread, nothing of the
        // system's audio touched.
        if let (Hosting::Waiting(_, renderer, _), Message::Wake) = (&mut *hosting, &message)
            && !renderer.idle()
        {
            return;
        }
        if let Hosting::Waiting(..) = &*hosting {
            let Hosting::Waiting(device, renderer, times) =
                std::mem::replace(&mut *hosting, Hosting::None)
            else {
                unreachable!()
            };
            let (messages, inbox) = std::sync::mpsc::channel();
            let (quitting, quit) = std::sync::mpsc::channel::<()>();
            let meter = self.0.meter.clone();
            // The stream closed and nothing to do: the device and renderer
            // back here, under this lock (which every message is sent under,
            // so none is missed), and the thread ends.
            let slot = Arc::downgrade(&self.0.hosting);
            let park: crate::device::Park = Box::new(move |device, renderer, inbox| {
                use crate::device::Parking;
                let Some(slot) = slot.upgrade() else {
                    return Parking::Kept(device, renderer, Message::Quit);
                };
                let mut hosting = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Ok(message) = inbox.try_recv() {
                    return Parking::Kept(device, renderer, message);
                }
                if let Hosting::Running(host) = &mut *hosting {
                    // Its thread is this one, ending: not waited for.
                    host.thread = None;
                }
                *hosting = Hosting::Waiting(device, renderer, times);
                Parking::Parked
            });
            let thread = std::thread::Builder::new()
                .name("zgx-audio".into())
                .spawn(move || {
                    crate::device::host(device, renderer, inbox, meter, times, park);
                    drop(quitting);
                })
                .expect("a thread for the audio device");
            *hosting = Hosting::Running(Host {
                messages,
                thread: Some(thread),
                quit,
            });
        }
        if let Hosting::Running(host) = &*hosting {
            let _ = host.messages.send(message);
        }
    }

    /// Starts the device now, before a sound needs it, and holds it for
    /// [`Config::hold`]: for an app that knows a sound is coming and would
    /// rather not wait for a stream to start.
    pub fn prewarm(&self) {
        self.wake(Message::Prewarm);
    }

    /// Why the device last failed to start, if it did.
    pub fn device_error(&self) -> Option<String> {
        self.0
            .meter
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// How many blocks took each number of microseconds to mix (the last
    /// entry: that long or longer), since the engine was made.
    pub fn block_times(&self) -> Vec<u32> {
        self.0
            .meter
            .times
            .iter()
            .map(|t| t.load(Ordering::Relaxed))
            .collect()
    }

    pub(crate) fn with_mixer(config: &Config) -> Result<(Audio, Mixer), Error> {
        config.check()?;
        let voices = config.voices;
        let (commands, from_main) = queue(config.commands);
        // Every sound (and effect) the mixer's side holds is in one place:
        // the command queue, the schedule (as much again), a voice's slot
        // (a bus's chain), or here.
        // Each call on the engine empties this queue, so it never holds
        // more than the rest can: it's never full, and the mixer gives
        // back a voice the moment it ends, whether the app calls or not.
        let held =
            commands.capacity() + config.commands + voices + FADING_SLOTS + MAX_BUSES * MAX_EFFECTS;
        let (to_main, returns) = queue(held);
        let meter = Arc::new(Meter::new(config.rate));
        let setup = Setup {
            rate: config.rate,
            voices,
            commands: config.commands,
            limiter: config.limiter,
            interpolation: config.interpolation,
        };
        let mixer = Mixer::new(&setup, from_main, to_main, meter.clone());
        let latency = mixer.latency();
        let buses = ["master", "ui", "music", "world"]
            .map(|name| (name.to_string(), 0))
            .to_vec();
        let audio = Audio(Arc::new(Inner {
            id: ENGINES.fetch_add(1, Ordering::Relaxed),
            control: Mutex::new(Control {
                commands,
                room: config.commands as u64,
                returns,
                next_id: 1,
                ended: Vec::new(),
                ui_sounds: true,
                buses,
                live: HashSet::new(),
                effects: Vec::new(),
            }),
            meter,
            latency,
            hosting: Arc::new(Mutex::new(Hosting::None)),
            decoder: Mutex::new(None),
            streamed: Default::default(),
            stream_memory: config.stream_memory,
        }));
        if config.gain != 1. {
            audio
                .send(
                    0,
                    Command::BusGain {
                        bus: 0,
                        value: config.gain,
                        ramp: Ramp::NONE,
                    },
                )
                .expect("a new queue has room");
        }
        for bus in 1..4 {
            audio
                .send(
                    0,
                    Command::AddBus {
                        bus,
                        parent: 0,
                        memory: BusMemory::new(),
                    },
                )
                .expect("a new queue has room for 16 commands");
        }
        Ok((audio, mixer))
    }

    fn control(&self) -> MutexGuard<'_, Control> {
        let mut control = self
            .0
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Sounds and effects the mixer is done with are dropped here, on
        // this thread.
        let mut again: Vec<(u64, Command)> = Vec::new();
        while let Some(back) = control.returns.pop() {
            match back {
                Return::Ended { id, sound } => {
                    control.live.remove(&id);
                    if control.ended.len() >= MAX_ENDED {
                        // No one has asked for a long while: the oldest half go.
                        control.ended.drain(..MAX_ENDED / 2);
                    }
                    control.ended.push(id);
                    drop(sound);
                }
                Return::Effect(slot) => {
                    control.effects.retain(|e| e.0 != slot.id);
                    drop(slot);
                }
                Return::Unprepared { bus, mut slot } => {
                    // Sent again only if it's still to stay on its bus: a
                    // removal sent meanwhile may have found nothing to
                    // take off, and it would run for ever.
                    let id = slot.id;
                    if control.effects.iter().any(|e| e.0 == id && !e.2) {
                        slot.prepare(self.rate());
                        again.push((id, Command::AddEffect { bus, slot }));
                    } else {
                        control.effects.retain(|e| e.0 != id);
                        drop(slot);
                    }
                }
            }
        }
        for (id, command) in again {
            // Sent again as soon as it can be; on a full queue it's lost,
            // as any command sent then would be, and off the books.
            if self.push(&mut control, 0, command).is_err() {
                control.effects.retain(|e| e.0 != id);
            }
        }
        control
    }

    fn send(&self, time: u64, command: Command) -> Result<(), Error> {
        self.push(&mut self.control(), time, command)
    }

    /// Queues `command` for frame `time` if the mixer has room for it,
    /// then wakes the device if it's stopped. What's sent and not yet done
    /// with never passes the schedule's room, so the mixer takes every
    /// command as it comes; a quarter of that is kept for commands due
    /// now, which a crowd of later ones can't take. The count goes up
    /// before the device's state is read, and the host stops it before
    /// reading the count: one of them sees the other.
    fn push(&self, control: &mut Control, time: u64, command: Command) -> Result<(), Error> {
        let meter = &self.0.meter;
        let waiting = meter.sent.load(Ordering::SeqCst) - meter.taken.load(Ordering::Acquire);
        let later = time > self.now().0;
        let room = if later {
            control.room - control.room / 4
        } else {
            control.room
        };
        if waiting >= room {
            return Err(Error::QueueFull);
        }
        control
            .commands
            .push((time, command))
            .map_err(|_| Error::QueueFull)?;
        let meter = &self.0.meter;
        meter.sent.fetch_add(1, Ordering::SeqCst);
        if !meter.running.load(Ordering::SeqCst) {
            self.wake(Message::Wake);
        }
        Ok(())
    }

    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.0.meter.rate.load(Ordering::Acquire) as u32
    }

    /// The frame the mixer will mix next.
    pub fn now(&self) -> Time {
        Time(self.0.meter.now.load(Ordering::Acquire))
    }

    /// `seconds` as frames, to add to a [`Time`].
    pub fn seconds(&self, seconds: f64) -> u64 {
        (seconds.max(0.) * f64::from(self.rate())).round() as u64
    }

    /// Frames between a command's time and its sound leaving the mixer
    /// (the limiter's look-ahead).
    pub fn latency(&self) -> usize {
        self.0.latency
    }

    /// Decodes a sound whole.
    pub fn load(&self, source: Source) -> Result<Sound, Error> {
        source.decode()
    }

    /// Plays `sound` as `play` says.
    /// A play's sends as the mixer takes them: buses of this engine only.
    fn sends_of(&self, play: &Play) -> Result<[Option<(u16, f32)>; crate::mixer::SENDS], Error> {
        let mut sends = [None; crate::mixer::SENDS];
        for (send, wanted) in sends.iter_mut().zip(play.sends) {
            if let Some((engine, bus, level)) = wanted {
                if engine != self.0.id {
                    return Err(Error::Foreign);
                }
                *send = Some((bus, level));
            }
        }
        Ok(sends)
    }

    pub fn play(&self, sound: &Sound, play: Play) -> Result<Voice, Error> {
        let frames = sound.frames() as u64;
        let region = match (&play.region, play.looping) {
            (Some(range), true) if range.start < range.end => Region {
                start: range.start.min(frames),
                end: range.end.min(frames),
                looping: true,
            },
            (_, looping) => Region {
                start: 0,
                end: frames,
                looping,
            },
        };
        let bus = match play.bus {
            None => 0,
            Some((engine, bus)) if engine == self.0.id => bus,
            Some(_) => return Err(Error::Foreign),
        };
        let mut control = self.control();
        let id = control.next_id;
        let command = Command::Play {
            id,
            sound: Feed::Sound(sound.0.clone()),
            bus,
            gain: gain(play.gain),
            pan: pan(play.pan),
            pitch: pitch(play.pitch),
            region,
            priority: play.priority,
            fade_in: play.fade_in,
            law: play.law,
            cutoff: cutoff(play.cutoff),
            sends: self.sends_of(&play)?,
            // Round its loop, however far; a one-shot's past its end ends
            // it, however far.
            offset: if region.looping && region.end > region.start && play.offset >= region.end {
                region.start + (play.offset - region.end) % (region.end - region.start)
            } else {
                play.offset
            }
            .min(u64::from(u32::MAX)) as u32,
        };
        let time = play.at.map_or(0, |at| at.0);
        self.push(&mut control, time, command)?;
        control.next_id += 1;
        control.live.insert(id);
        Ok(Voice {
            audio: self.clone(),
            id,
            start: time,
        })
    }

    /// Plays `source` as it decodes, on the decode thread, about half a
    /// second ahead: for music and anything long. `play` says how, as for a
    /// sound; `looping` loops the whole stream, seamlessly. A stream that
    /// runs dry (its decoding fell behind) fades out over 5 ms, counts in
    /// [`Stats::dry`], and fades back in when frames come.
    pub fn stream(&self, source: Source, play: Play) -> Result<Voice, Error> {
        let decode = source.decoder()?;
        let (channels, rate) = (decode.channels(), decode.rate());
        let bus = match play.bus {
            None => 0,
            Some((engine, bus)) if engine == self.0.id => bus,
            Some(_) => return Err(Error::Foreign),
        };
        // Its ring and its window, of the streams' budget.
        let ring_samples = rate as usize / 2 * channels;
        let bytes = (ring_samples + crate::mixer::WINDOW * channels) * size_of::<f32>();
        let charge = crate::mixer::Charge::take(&self.0.streamed, bytes, self.0.stream_memory)
            .map_err(|free| Error::StreamMemory { asked: bytes, free })?;
        let (writer, reader) = ring(ring_samples);
        let streamed = Box::new(Streamed::new(reader, channels, rate, charge));
        let mut control = self.control();
        let id = control.next_id;
        let command = Command::Play {
            id,
            sound: Feed::Stream(streamed),
            bus,
            gain: gain(play.gain),
            pan: pan(play.pan),
            pitch: pitch(play.pitch),
            region: Region {
                start: 0,
                end: u64::MAX,
                looping: false,
            },
            priority: play.priority,
            fade_in: play.fade_in,
            law: play.law,
            cutoff: cutoff(play.cutoff),
            sends: self.sends_of(&play)?,
            offset: 0,
        };
        let time = play.at.map_or(0, |at| at.0);
        // The decode thread starts filling before the mixer hears of it.
        self.0
            .decoder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert_with(Decoder::new)
            .open(Feeding {
                decode,
                writer,
                looping: play.looping,
                skip: play.offset,
            });
        self.push(&mut control, time, command)?;
        control.next_id += 1;
        control.live.insert(id);
        Ok(Voice {
            audio: self.clone(),
            id,
            start: time,
        })
    }

    /// Voices that have ended (by their [`Voice::id`]) since this was last
    /// asked: what turns a `playing` state false. An app that never asks
    /// doesn't keep them all: past 65 536, the oldest are let go
    /// ([`Voice::playing`] still knows).
    pub fn take_ended(&self) -> Vec<u64> {
        std::mem::take(&mut self.control().ended)
    }

    /// Voices playing or due, as of the mixer's last block.
    pub fn voices_playing(&self) -> usize {
        self.control().live.len()
    }

    /// Calls `notify` (from the engine's own thread, never the audio
    /// thread) when voices end while a device runs, so an app asleep in its
    /// event loop wakes to call [`take_ended`](Audio::take_ended). An
    /// offline engine has no device: its caller asks after rendering.
    pub fn on_voices_ended(&self, notify: impl Fn() + Send + 'static) {
        *self
            .0
            .meter
            .notify
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(notify));
    }

    /// Follows the user's interface-sound setting: with it off, the `ui`
    /// bus is silent (over 5 ms) and every other bus plays on. Apart from
    /// the app's own mute: neither undoes the other.
    pub fn set_ui_sounds(&self, enabled: bool) -> Result<(), Error> {
        let mut control = self.control();
        control.ui_sounds = enabled;
        self.push(
            &mut control,
            0,
            Command::Quiet {
                bus: UI_BUS,
                on: !enabled,
            },
        )
    }

    /// Whether interface sounds play, as last set by
    /// [`set_ui_sounds`](Audio::set_ui_sounds) (true until then).
    pub fn ui_sounds_enabled(&self) -> bool {
        self.control().ui_sounds
    }

    /// Waits until the decode thread has filled every stream as far as it
    /// can. [`Offline::render`] does it before each block.
    pub fn settle(&self) {
        if let Some(decoder) = &*self
            .0
            .decoder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            decoder.fill();
        }
    }

    /// Drops the sounds the mixer has finished with. Every other call does
    /// it too.
    pub fn update(&self) {
        drop(self.control());
    }

    /// The master bus: every other bus leads to it.
    pub fn master(&self) -> Bus {
        Bus {
            audio: self.clone(),
            index: 0,
            time: 0,
        }
    }

    /// The bus named `name`: `master`, `ui`, `music` and `world` are there
    /// from the start.
    pub fn bus(&self, name: &str) -> Option<Bus> {
        let control = self.control();
        let index = control.buses.iter().position(|(bus, _)| bus == name)?;
        Some(Bus {
            audio: self.clone(),
            index: index as u16,
            time: 0,
        })
    }

    /// A new bus feeding `parent`, one of this engine's.
    pub fn add_bus(&self, name: &str, parent: &Bus) -> Result<Bus, Error> {
        if parent.audio.0.id != self.0.id {
            return Err(Error::Foreign);
        }
        let mut control = self.control();
        if control.buses.iter().any(|(bus, _)| bus == name) {
            return Err(Error::BusExists(name.into()));
        }
        if control.buses.len() >= MAX_BUSES {
            return Err(Error::TooManyBuses);
        }
        let index = control.buses.len() as u16;
        // Its buffers made here, not on the audio thread: a bus never added
        // costs none.
        let command = Command::AddBus {
            bus: index,
            parent: parent.index,
            memory: BusMemory::new(),
        };
        self.push(&mut control, 0, command)?;
        control.buses.push((name.into(), parent.index));
        Ok(Bus {
            audio: self.clone(),
            index,
            time: 0,
        })
    }

    /// What the mixer has done.
    pub fn stats(&self) -> Stats {
        let meter = &self.0.meter;
        Stats {
            frames: meter.now.load(Ordering::Acquire),
            blocks: meter.blocks.load(Ordering::Relaxed),
            voices: meter.voices.load(Ordering::Relaxed),
            stolen: meter.stolen.load(Ordering::Relaxed),
            dropped: meter.dropped.load(Ordering::Relaxed),
            late: meter.late.load(Ordering::Relaxed),
            commands: meter.sent.load(Ordering::SeqCst),
            sounding: meter.sounding.load(Ordering::Relaxed),
            running: meter.running.load(Ordering::SeqCst),
            starts: meter.starts.load(Ordering::Relaxed),
            stops: meter.stops.load(Ordering::Relaxed),
            start_micros: meter.start_micros.load(Ordering::Relaxed),
            losses: meter.losses.load(Ordering::Relaxed),
            failures: meter.failures.load(Ordering::Relaxed),
            underruns: meter.underruns.load(Ordering::Relaxed),
            dry: meter.dry.load(Ordering::Relaxed),
            stream_memory: self.0.streamed.load(Ordering::Acquire) as u64,
            streams: self
                .0
                .decoder
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map_or(0, |d| d.open.load(Ordering::Acquire) as u64),
            block_asked: meter.block_asked.load(Ordering::Relaxed),
            block_heard: meter.block_heard.load(Ordering::Relaxed),
            play_heard: meter.play_heard.load(Ordering::Acquire),
            play_asked: meter.play_asked.load(Ordering::Relaxed),
            play_mixed: crate::device::clock::nanos(meter.play_called.load(Ordering::Relaxed)),
            peak: f32::from_bits(meter.peak.load(Ordering::Relaxed)),
            waited: meter.waited.load(Ordering::Relaxed),
        }
    }
}

/// How a sound is played: the defaults are master, whole gain, centred,
/// its own pitch, now, once.
#[derive(Clone, Debug)]
pub struct Play {
    /// The bus's engine, and the bus.
    bus: Option<(u64, u16)>,
    gain: f32,
    pan: f32,
    pitch: f32,
    at: Option<Time>,
    looping: bool,
    region: Option<Range<u64>>,
    priority: i8,
    fade_in: Option<Ramp>,
    law: PanLaw,
    cutoff: f32,
    /// Buses it's also mixed into, each with its engine and level.
    sends: [Option<(u64, u16, f32)>; crate::mixer::SENDS],
    /// Its sound's frame it starts on.
    offset: u64,
}

impl Default for Play {
    fn default() -> Self {
        Play::new()
    }
}

impl Play {
    pub fn new() -> Play {
        Play {
            bus: None,
            gain: 1.,
            pan: 0.,
            pitch: 1.,
            at: None,
            looping: false,
            region: None,
            priority: 0,
            fade_in: None,
            law: PanLaw::Balance,
            cutoff: crate::effects::VOICE_OPEN,
            sends: [None; crate::mixer::SENDS],
            offset: 0,
        }
    }
    /// Starts `frames` into the sound (its own frames, at its own rate):
    /// partway through. A looping sound starts that far round its loop,
    /// however far; a one-shot started past its end ends at once. A stream
    /// decodes its way there, dropping what comes before.
    pub fn offset(mut self, frames: u64) -> Play {
        self.offset = frames;
        self
    }
    /// The bus it plays into: one of the engine's it's played on (another
    /// engine's is an error).
    pub fn bus(mut self, bus: &Bus) -> Play {
        self.bus = Some((bus.audio.0.id, bus.index));
        self
    }
    /// Its gain, 1 as recorded, kept from 0 to [`MAX_GAIN`].
    pub fn gain(mut self, gain: f32) -> Play {
        self.gain = gain;
        self
    }
    /// From -1 (left) to 1 (right), as a balance: the near side stays whole.
    pub fn pan(mut self, pan: f32) -> Play {
        self.pan = pan;
        self
    }
    /// Speed and pitch together: 2 is an octave up and twice as fast.
    /// Kept from 1/64 to 16.
    pub fn pitch(mut self, pitch: f32) -> Play {
        self.pitch = pitch;
        self
    }
    /// Starts on exactly this frame of the mixer's clock.
    pub fn at(mut self, time: Time) -> Play {
        self.at = Some(time);
        self
    }
    /// Plays round until stopped.
    pub fn looping(mut self, looping: bool) -> Play {
        self.looping = looping;
        self
    }
    /// Loops these frames of the sound (at its own rate), after playing up
    /// to them once: an intro, then the loop.
    pub fn loop_frames(mut self, frames: Range<u64>) -> Play {
        self.looping = true;
        self.region = Some(frames);
        self
    }
    /// How its pan shares it between left and right: a balance (the
    /// default) or equal power (for sounds in space).
    pub fn law(mut self, law: PanLaw) -> Play {
        self.law = law;
        self
    }
    /// Starts with its own low-pass at `hz` ([`Voice::set_cutoff`]).
    pub fn cutoff(mut self, hz: f32) -> Play {
        self.cutoff = hz;
        self
    }
    /// Mixed into `bus` too, at `level` (a gain, 0 to [`MAX_GAIN`]), from
    /// its first frame: a send ([`Voice::set_send`]). Two buses at most, as
    /// there: a send to a third is ignored; another to a bus it sends to
    /// already sets that one's level.
    pub fn send(mut self, bus: &Bus, level: f32) -> Play {
        let (engine, index) = (bus.audio.0.id, bus.index);
        let send = Some((engine, index, gain(level)));
        let same = self
            .sends
            .iter()
            .position(|s| s.is_some_and(|(e, b, _)| (e, b) == (engine, index)));
        if let Some(at) = same.or_else(|| self.sends.iter().position(Option::is_none)) {
            self.sends[at] = send;
        }
        self
    }
    /// Kept over voices of lower priority when there are too many.
    pub fn priority(mut self, priority: i8) -> Play {
        self.priority = priority;
        self
    }
    /// Fades in from silence. Without one, a sound that starts away from
    /// zero fades in over a millisecond, so it never clicks.
    pub fn fade_in(mut self, ramp: impl Into<Ramp>) -> Play {
        self.fade_in = Some(ramp.into());
        self
    }
}

impl From<f64> for Ramp {
    /// That many seconds, in straight steps.
    fn from(seconds: f64) -> Ramp {
        Ramp::seconds(seconds)
    }
}

/// The `ui` bus's index: made second, after master.
const UI_BUS: u16 = 1;

/// A sound playing, or due to.
#[derive(Clone, Debug)]
pub struct Voice {
    audio: Audio,
    id: u64,
    /// The frame it was asked to start on (0: as soon as it could): what
    /// it's told after starts then, not before.
    start: u64,
}

impl Voice {
    fn send(&self, command: Command) -> Result<(), Error> {
        self.audio.send(self.start, command)
    }

    /// The same voice, for commands that happen on exactly this frame:
    /// `voice.at(audio.now() + audio.seconds(1.)).stop()`.
    pub fn at(&self, time: Time) -> Voice {
        Voice {
            start: self.start.max(time.0),
            ..self.clone()
        }
    }

    /// Stops it with a 5 ms fade.
    pub fn stop(&self) -> Result<(), Error> {
        self.stop_after(Ramp::ms(5.))
    }

    /// Fades it out and stops it. A fade, never a cut: at least a
    /// millisecond. A voice not yet heard stops without a sound.
    pub fn stop_after(&self, fade: impl Into<Ramp>) -> Result<(), Error> {
        let fade = fade.into();
        self.send(Command::Stop {
            id: self.id,
            ramp: fade,
        })
    }

    fn set(&self, param: VoiceParam, value: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        let ramp = ramp.into();
        let value = match param {
            VoiceParam::Gain => gain(value),
            VoiceParam::Pan => pan(value),
            VoiceParam::Pitch => pitch(value),
            VoiceParam::Cutoff => cutoff(value),
        };
        self.send(Command::Voice {
            id: self.id,
            param,
            value,
            ramp,
        })
    }

    /// Moves its gain to `gain`, kept from 0 to [`MAX_GAIN`].
    pub fn set_gain(&self, gain: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        self.set(VoiceParam::Gain, gain, ramp)
    }

    /// Moves its pan to `pan`, in straight steps whatever the curve.
    pub fn set_pan(&self, pan: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        self.set(VoiceParam::Pan, pan, ramp)
    }

    /// Moves its pitch to `pitch`.
    pub fn set_pitch(&self, pitch: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        self.set(VoiceParam::Pitch, pitch, ramp)
    }

    /// Mixes it into `bus` too, besides its own, at `level` (a gain, 0 to
    /// [`MAX_GAIN`]) along `ramp`: a send. Its sound after its gain, pan
    /// and own low-pass goes there, scaled: to a bus with a reverb on it,
    /// a binaural effect, or anything of the app's. A voice sends to two
    /// buses at most: a send to a third is ignored until one of the two is
    /// brought to 0, which takes it away. A voice with no sends costs
    /// nothing more for them.
    pub fn set_send(&self, bus: &Bus, level: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        if bus.audio.0.id != self.audio.0.id {
            return Err(Error::Foreign);
        }
        self.send(Command::Send {
            id: self.id,
            bus: bus.index,
            level: gain(level),
            ramp: ramp.into(),
        })
    }

    /// Moves its own low-pass's cutoff to `hz` (12 dB an octave, flat; kept
    /// from 10 Hz to 20 kHz, where it's no filter): for what's between a
    /// sound and its listener, a wall or the air. Before its gain and pan,
    /// apart from its bus's effects. A voice whose cutoff stays at 20 kHz
    /// runs none.
    pub fn set_cutoff(&self, hz: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        self.set(VoiceParam::Cutoff, hz, ramp)
    }

    /// The id [`Audio::take_ended`] gives back when it ends.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Whether it's playing or due to, as of the mixer's last block.
    pub fn playing(&self) -> bool {
        self.audio.control().live.contains(&self.id)
    }
}

/// A bus: a sum of voices and other buses, with a gain, a mute and a solo.
#[derive(Clone, Debug)]
pub struct Bus {
    audio: Audio,
    index: u16,
    /// The frame its commands happen on (0: as soon as they can).
    time: u64,
}

impl Bus {
    /// The same bus, for commands that happen on exactly this frame.
    pub fn at(&self, time: Time) -> Bus {
        Bus {
            time: time.0,
            ..self.clone()
        }
    }

    /// Moves its gain to `gain` (kept from 0 to [`MAX_GAIN`]): a fade, with
    /// a [`Ramp`]'s curve.
    pub fn set_gain(&self, gain: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        let ramp = ramp.into();
        self.audio.send(
            self.time,
            Command::BusGain {
                bus: self.index,
                value: self::gain(gain),
                ramp,
            },
        )
    }

    /// Fades it to `gain` over `seconds` along `curve`.
    pub fn fade_to(&self, gain: f32, seconds: f64, curve: Curve) -> Result<(), Error> {
        self.set_gain(gain, Ramp::seconds(seconds).curve(curve))
    }

    /// A handle that sets this bus's gain with no command: the mixer reads
    /// it each block and glides to it over 10 ms. For a slider or an
    /// animation that changes every frame; a later [`set_gain`](Bus::set_gain)
    /// takes over until the drive is set again.
    pub fn drive(&self) -> BusDrive {
        BusDrive {
            meter: self.audio.0.meter.clone(),
            index: usize::from(self.index),
        }
    }

    /// Silences it, over 5 ms.
    pub fn mute(&self, on: bool) -> Result<(), Error> {
        self.audio.send(
            self.time,
            Command::Mute {
                bus: self.index,
                on,
            },
        )
    }

    /// Silences every bus that isn't soloed and isn't on the way to or
    /// from one that is, over 5 ms.
    pub fn solo(&self, on: bool) -> Result<(), Error> {
        self.audio.send(
            self.time,
            Command::Solo {
                bus: self.index,
                on,
            },
        )
    }

    /// Its name.
    pub fn name(&self) -> String {
        self.audio.control().buses[usize::from(self.index)]
            .0
            .clone()
    }

    /// Adds `effect` at the end of its chain: it runs on the bus's sum,
    /// after the effects before it, before the bus's gain. Prepared here,
    /// on this thread, then sent to the mixer.
    pub fn add_effect<E: Effect>(&self, effect: E) -> Result<EffectHandle, Error> {
        self.add(effect, None)
    }

    /// As [`add_effect`](Bus::add_effect), with `side`'s level given to the
    /// effect each block ([`Context::side_chain`](crate::Context::side_chain)):
    /// that bus's level over the block before, so neither bus waits on the
    /// other. `side` must be this engine's.
    pub fn add_effect_keyed<E: Effect>(
        &self,
        effect: E,
        side: &Bus,
    ) -> Result<EffectHandle, Error> {
        if side.audio.0.id != self.audio.0.id {
            return Err(Error::Foreign);
        }
        self.add(effect, Some(side.index))
    }

    /// Ducks this bus under `side` (music under dialogue): `side`'s level
    /// drives a [`Duck`](crate::effects::Duck) at the end of this bus's
    /// chain.
    /// ```
    /// # use zgx_audio::{Audio, Config, effects::Duck};
    /// let (audio, _mixer) = Audio::offline(Config::default())?;
    /// let (music, dialogue) = (audio.bus("music").unwrap(), audio.add_bus("dialogue", &audio.master())?);
    /// music.duck(&dialogue, Duck::new(-9., 0.05, 0.4))?;
    /// # Ok::<(), zgx_audio::Error>(())
    /// ```
    pub fn duck(&self, side: &Bus, duck: crate::effects::Duck) -> Result<EffectHandle, Error> {
        self.add_effect_keyed(duck, side)
    }

    fn add<E: Effect>(&self, effect: E, side: Option<u16>) -> Result<EffectHandle, Error> {
        // Its parameters as the mixer will keep to them: a range the right
        // way up, all of it numbers. Checked here: the mixer can't refuse.
        for p in effect.parameters() {
            if !(p.min.is_finite() && p.max.is_finite() && p.min <= p.max && !p.default.is_nan()) {
                return Err(Error::Effect(format!(
                    "a parameter {:?} from {} to {}, starting at {}",
                    p.name, p.min, p.max, p.default
                )));
            }
        }
        let mut control = self.audio.control();
        let held = control.effects.iter().filter(|e| e.1 == self.index).count();
        if held >= MAX_EFFECTS {
            return Err(Error::TooManyEffects);
        }
        let id = control.next_id;
        let slot = Box::new(Slot::new(id, effect, self.audio.rate(), side));
        let command = Command::AddEffect {
            bus: self.index,
            slot,
        };
        self.audio.push(&mut control, self.time, command)?;
        control.next_id += 1;
        control.effects.push((id, self.index, false));
        Ok(EffectHandle {
            audio: self.audio.clone(),
            id,
            time: self.time,
            added: self.time,
        })
    }
}

/// An effect on a bus: its parameters, and taking it off again.
#[derive(Clone, Debug)]
pub struct EffectHandle {
    audio: Audio,
    id: u64,
    /// The frame its commands happen on (0: as soon as they can).
    time: u64,
    /// The frame it was added on: its commands come no earlier, or they'd
    /// find nothing, and a removal would leave it to run for ever.
    added: u64,
}

impl EffectHandle {
    /// The same effect, for commands that happen on exactly this frame:
    /// one before the frame it was added on is the frame it was added on.
    pub fn at(&self, time: Time) -> EffectHandle {
        EffectHandle {
            time: time.0.max(self.added),
            ..self.clone()
        }
    }

    /// Moves parameter `index` to `value`, kept to its range, along `ramp`:
    /// the effect sees each frame's value. An index the effect hasn't is
    /// ignored. The curves for gains ([`Curve::EqualPower`],
    /// [`Curve::Exponential`]) take values as gains, none below zero: a
    /// parameter whose range goes below zero (decibels, mostly) moves in
    /// straight steps whatever the ramp's curve.
    pub fn set(&self, index: usize, value: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        let ramp = ramp.into();
        self.audio.send(
            self.time,
            Command::EffectParam {
                id: self.id,
                index: index.min(usize::from(u16::MAX)) as u16,
                value,
                ramp,
            },
        )
    }

    /// Takes it off its bus, at once: its tail is cut. It's dropped on
    /// this thread by a later call. Taken off at a later frame, it holds
    /// its place on the bus until then.
    pub fn remove(&self) -> Result<(), Error> {
        let mut control = self.audio.control();
        let later = self.time > self.audio.now().0;
        self.audio.push(
            &mut control,
            self.time,
            Command::RemoveEffect { id: self.id },
        )?;
        if later {
            // Off the books when the mixer gives it back.
            for e in control.effects.iter_mut().filter(|e| e.0 == self.id) {
                e.2 = true;
            }
        } else {
            // The mixer takes it off before anything sent after.
            control.effects.retain(|e| e.0 != self.id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_gain_engine_mixes_but_sends_silence_from_its_first_frame() {
        let loud = Sound::from_samples(&[0.9; 4800], 1, 48_000).unwrap();
        let (audio, mut mixer) = Audio::offline(Config {
            gain: 0.,
            ..Config::default()
        })
        .unwrap();
        audio.play(&loud, Play::new().gain(4.)).unwrap();
        let out = mixer.frames(4800);
        assert!(out.iter().all(|&s| s == 0.));
        let stats = audio.stats();
        assert_eq!(stats.peak, 0.);
        assert!(stats.sounding >= 4700, "it mixed: {stats:?}");
    }

    #[test]
    fn sounds_are_dropped_by_the_main_thread_once_played() {
        let click = Sound::from_samples(&[0., 1., 0.5], 1, 48_000).unwrap();
        let (audio, mut mixer) = Audio::offline(Config::default()).unwrap();
        let voice = audio.play(&click, Play::new()).unwrap();
        assert!(voice.playing());
        assert_eq!(Arc::strong_count(&click.0), 2, "the command holds it");
        mixer.frames(64);
        assert_eq!(
            Arc::strong_count(&click.0),
            2,
            "the mixer gave it back, still alive"
        );
        assert_eq!(audio.stats().voices, 0);
        audio.update();
        assert!(!voice.playing());
        assert_eq!(Arc::strong_count(&click.0), 1);
    }

    #[test]
    fn an_effect_prepared_for_a_rate_gone_by_is_prepared_again_and_sent_back() {
        struct Doubles(Arc<Mutex<Vec<u32>>>);
        impl Effect for Doubles {
            fn prepare(&mut self, format: crate::Format, _: usize) {
                self.0.lock().unwrap().push(format.rate);
            }
            fn process(&mut self, block: &mut [f32], _: &crate::Context<'_>) {
                block.iter_mut().for_each(|s| *s *= 2.);
            }
        }
        let flat = Sound::from_samples(&[0., 0.25], 1, 48_000).unwrap();
        let (audio, mut mixer) = Audio::offline(Config {
            limiter: None,
            ..Config::default()
        })
        .unwrap();
        audio.play(&flat, Play::new().loop_frames(1..2)).unwrap();
        // The rate the main thread reads moves under it, as a device's
        // starting at another rate would move it.
        let prepared = Arc::new(Mutex::new(Vec::new()));
        audio.0.meter.rate.store(44_100, Ordering::Release);
        audio
            .master()
            .add_effect(Doubles(prepared.clone()))
            .unwrap();
        audio.0.meter.rate.store(48_000, Ordering::Release);
        // Not run at the wrong rate: given back...
        assert_eq!(mixer.frames(64)[2], 0.25);
        // ...prepared again here, and sent again.
        audio.update();
        assert_eq!(mixer.frames(64)[2], 0.5);
        assert_eq!(*prepared.lock().unwrap(), [44_100, 48_000]);
    }

    #[test]
    fn an_effect_taken_off_before_a_change_of_rate_reached_it_stays_off() {
        struct Doubles(Arc<std::sync::atomic::AtomicBool>);
        impl Effect for Doubles {
            fn prepare(&mut self, _: crate::Format, _: usize) {}
            fn process(&mut self, block: &mut [f32], _: &crate::Context<'_>) {
                block.iter_mut().for_each(|s| *s *= 2.);
            }
        }
        impl Drop for Doubles {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let flat = Sound::from_samples(&[0., 0.25], 1, 48_000).unwrap();
        let (audio, mut mixer) = Audio::offline(Config {
            limiter: None,
            ..Config::default()
        })
        .unwrap();
        audio.play(&flat, Play::new().loop_frames(1..2)).unwrap();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        audio.0.meter.rate.store(44_100, Ordering::Release);
        let effect = audio.master().add_effect(Doubles(dropped.clone())).unwrap();
        audio.0.meter.rate.store(48_000, Ordering::Release);
        // Taken off before the mixer saw it: the mixer gives it back to be
        // made again, then finds nothing to take off.
        effect.remove().unwrap();
        assert_eq!(mixer.frames(64)[2], 0.25);
        // So it isn't sent again, to run for ever: it's dropped here.
        audio.update();
        assert_eq!(mixer.frames(64)[2], 0.25, "it runs after its removal");
        audio.update();
        assert!(dropped.load(Ordering::Relaxed));
        // And its place on the bus is free.
        for _ in 0..MAX_EFFECTS {
            audio.master().add_effect(Doubles(Arc::default())).unwrap();
        }
    }

    /// An offline engine with room for 2^17 commands, the 64 voices of
    /// which have played and ended, and `plays` plays due at frame 1024.
    fn crowded(plays: usize) -> (Audio, Offline, Vec<Voice>) {
        let (audio, mut mixer) = Audio::offline(Config {
            commands: 131_072,
            limiter: None,
            ..Config::default()
        })
        .unwrap();
        let blip = Sound::from_samples(&[0., 0.5], 1, 48_000).unwrap();
        let ended = (0..64)
            .map(|_| audio.play(&blip, Play::new()).unwrap())
            .collect();
        mixer.frames(256);
        let long = Sound::from_samples(&[0.25; 48_000], 1, 48_000).unwrap();
        for _ in 0..plays {
            audio.play(&long, Play::new().at(Time(1_024))).unwrap();
        }
        (audio, mixer, ended)
    }

    fn work(mixer: &Offline) -> [u64; 2] {
        mixer
            .renderer
            .mixer
            .work
            .each_ref()
            .map(std::cell::Cell::get)
    }

    #[test]
    fn a_crowd_of_plays_waiting_for_voices_is_tried_from_its_head() {
        // Far more plays due on one frame than there are voices: those
        // that find no slot wait. Each block while they wait tries the
        // first, and those that then find slots, not every one of them.
        let (_audio, mut mixer, _) = crowded(90_000);
        // The block they come due in looks at each once, as any command due.
        mixer.frames(1_024 + 128 - 256);
        let before = work(&mixer)[0];
        mixer.frames(128 * 40);
        let tried = work(&mixer)[0] - before;
        assert!(tried < 2_000, "{tried} plays tried in 40 blocks");
    }

    #[test]
    fn commands_for_ended_voices_search_nothing_however_many_plays_wait() {
        let (audio, mut mixer, ended) = crowded(90_000);
        mixer.frames(1_024 + 128 - 256);
        let before = work(&mixer)[1];
        for voice in &ended {
            let _ = voice.set_gain(0.5, Ramp::NONE);
        }
        mixer.frames(128);
        assert_eq!(work(&mixer)[1] - before, 0, "searched for waiting plays");
        drop(audio);
    }

    #[test]
    fn a_config_an_engine_can_t_be_made_with_is_an_error_not_a_panic() {
        let bad = [
            Config {
                limiter: Some(-1.),
                ..Config::default()
            },
            Config {
                limiter: Some(f32::NAN),
                ..Config::default()
            },
            Config {
                limiter: Some(f32::INFINITY),
                ..Config::default()
            },
            Config {
                limiter: Some(0.),
                ..Config::default()
            },
            Config {
                commands: 1,
                ..Config::default()
            },
            Config {
                commands: 2,
                ..Config::default()
            },
            Config {
                voices: usize::MAX,
                ..Config::default()
            },
            Config {
                voices: 0,
                ..Config::default()
            },
            Config {
                rate: 0,
                ..Config::default()
            },
            Config {
                rate: u32::MAX,
                ..Config::default()
            },
            Config {
                channels: 0,
                ..Config::default()
            },
            Config {
                gain: f32::NAN,
                ..Config::default()
            },
            Config {
                gain: -1.,
                ..Config::default()
            },
        ];
        for config in bad {
            let error = Audio::offline(config.clone()).err();
            assert!(
                matches!(error, Some(Error::Config(_))),
                "{config:?}: {error:?}"
            );
        }
        // At their limits, they make engines that mix.
        let edge = Config {
            rate: 1_000,
            channels: 64,
            voices: 1,
            commands: 16,
            limiter: Some(f32::MIN_POSITIVE),
            ..Config::default()
        };
        let (audio, mut mixer) = Audio::offline(edge).unwrap();
        let click = Sound::from_samples(&[0., 1., 0.5], 1, 1_000).unwrap();
        audio.play(&click, Play::new()).unwrap();
        assert_eq!(mixer.frames(64).len(), 64 * 64);
    }
}

/// A bus's gain, set from any thread at any rate without a command: see
/// [`Bus::drive`].
#[derive(Clone)]
pub struct BusDrive {
    meter: Arc<Meter>,
    index: usize,
}

impl std::fmt::Debug for BusDrive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusDrive")
            .field("bus", &self.index)
            .finish()
    }
}

impl BusDrive {
    /// The gain the bus glides to, kept from 0 to [`MAX_GAIN`] (not a
    /// number: 0).
    pub fn set(&self, gain: f32) {
        let gain = self::gain(gain);
        self.meter.drives[self.index].store(gain.to_bits(), Ordering::Relaxed);
    }
}
