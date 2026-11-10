//! The main thread's side: [`Audio`] and the handles it gives out. Each
//! call becomes a command stamped with a frame and goes on a fixed queue;
//! a full queue is an error, never a wait.
use crate::device::{Device, Host, Message, Renderer};
use crate::mixer::{Command, MAX_BUSES, Meter, Mixer, Return, Setup, VoiceParam};
use crate::queue::{Consumer, Producer, queue};
use crate::ramp::{Curve, Ramp};
use crate::resample::{Interpolation, Region};
use crate::sound::{Sound, Source};
use std::collections::HashSet;
use std::ops::{Add, Range, Sub};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The command queue is full: the mixer hasn't run for a while, or
    /// more was sent in one go than [`Config::commands`] holds.
    QueueFull,
    /// [`Audio::add_bus`] past the most buses there can be (32).
    TooManyBuses,
    /// A bus of this name is there already.
    BusExists(String),
    /// Bytes that aren't a sound zgx-audio reads, and why.
    Format(String),
    /// A file that couldn't be read.
    Io(String),
    /// The output device couldn't be started, and why.
    Device(String),
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
            Error::BusExists(name) => write!(f, "a bus named {name:?} exists already"),
            Error::Format(why) | Error::Io(why) => f.write_str(why),
            Error::Device(why) => write!(f, "the audio device: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// A frame on the mixer's clock, counted from its first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(pub u64);

impl Add<u64> for Time {
    type Output = Time;
    fn add(self, frames: u64) -> Time {
        Time(self.0 + frames)
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
}

struct Control {
    commands: Producer<(u64, Command)>,
    returns: Consumer<Return>,
    next_id: u64,
    /// Name and parent of each bus, by index.
    buses: Vec<(String, u16)>,
    /// Voices sent and not yet given back.
    live: HashSet<u64>,
}

/// The device and its host thread, made when first needed.
enum Hosting {
    /// No device: an offline engine.
    None,
    /// A device not started yet, and the renderer it will run.
    Waiting(Box<dyn Device>, Box<Renderer>, Duration),
    Running(Host),
}

struct Inner {
    control: Mutex<Control>,
    meter: Arc<Meter>,
    latency: usize,
    hosting: Mutex<Hosting>,
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
}

impl Offline {
    /// Mixes the next `out.len() / channels` frames into `out`.
    pub fn render(&mut self, out: &mut [f32]) {
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
    pub fn offline(config: Config) -> (Audio, Offline) {
        let (audio, mixer) = Audio::with_mixer(&config);
        let channels = config.channels.max(1);
        let renderer = Renderer::new(mixer, channels, audio.0.meter.clone());
        (audio, Offline { renderer, channels })
    }

    /// An engine on the system's default output, through cpal. Nothing is
    /// opened until the first sound: then a stream starts, and it stops
    /// after [`Config::hold`] with nothing to hear.
    #[cfg(feature = "device")]
    pub fn new(config: Config) -> Audio {
        Audio::with_device(config, crate::Cpal::new())
    }

    /// An engine on `device`, started with the first sound.
    pub fn with_device(config: Config, device: impl Device) -> Audio {
        let channels = config.channels;
        let hold = config.hold;
        let (audio, mixer) = Audio::with_mixer(&config);
        let renderer = Box::new(Renderer::new(mixer, channels, audio.0.meter.clone()));
        *audio.hosting() = Hosting::Waiting(Box::new(device), renderer, hold);
        audio
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
        if let Hosting::Waiting(..) = &*hosting {
            let Hosting::Waiting(device, renderer, hold) =
                std::mem::replace(&mut *hosting, Hosting::None)
            else {
                unreachable!()
            };
            let (messages, inbox) = std::sync::mpsc::channel();
            let meter = self.0.meter.clone();
            let thread = std::thread::Builder::new()
                .name("zgx-audio".into())
                .spawn(move || crate::device::host(device, renderer, inbox, meter, hold))
                .expect("a thread for the audio device");
            *hosting = Hosting::Running(Host {
                messages,
                thread: Some(thread),
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

    pub(crate) fn with_mixer(config: &Config) -> (Audio, Mixer) {
        let voices = config.voices.max(1);
        let (commands, from_main) = queue(config.commands.max(1));
        let (to_main, returns) = queue(voices * 2 + 64);
        let meter = Arc::new(Meter::new(config.rate.max(1)));
        let setup = Setup {
            rate: config.rate.max(1),
            voices,
            commands: config.commands.max(1),
            limiter: config.limiter,
            interpolation: config.interpolation,
        };
        let mixer = Mixer::new(&setup, from_main, to_main, meter.clone());
        let latency = mixer.latency();
        let buses = ["master", "ui", "music", "world"]
            .map(|name| (name.to_string(), 0))
            .to_vec();
        let audio = Audio(Arc::new(Inner {
            control: Mutex::new(Control {
                commands,
                returns,
                next_id: 1,
                buses,
                live: HashSet::new(),
            }),
            meter,
            latency,
            hosting: Mutex::new(Hosting::None),
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
                .send(0, Command::AddBus { bus, parent: 0 })
                .expect("a new queue has room");
        }
        (audio, mixer)
    }

    fn control(&self) -> MutexGuard<'_, Control> {
        let mut control = self
            .0
            .control
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Sounds the mixer is done with are dropped here, on this thread.
        while let Some(Return::Ended { id, sound }) = control.returns.pop() {
            control.live.remove(&id);
            drop(sound);
        }
        control
    }

    fn send(&self, time: u64, command: Command) -> Result<(), Error> {
        self.push(&mut self.control(), time, command)
    }

    /// Queues `command`, then wakes the device if it's stopped. The count
    /// goes up before the device's state is read, and the host stops it
    /// before reading the count: one of them sees the other.
    fn push(&self, control: &mut Control, time: u64, command: Command) -> Result<(), Error> {
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
        let sound = source.decode()?;
        if sound.frames() as u64 >= u64::from(u32::MAX) {
            return Err(Error::Format(
                "a sound of over 2^32 frames: stream it instead".into(),
            ));
        }
        Ok(sound)
    }

    /// Plays `sound` as `play` says.
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
        let mut control = self.control();
        let id = control.next_id;
        let bus = play.bus.unwrap_or(0);
        let command = Command::Play {
            id,
            sound: sound.0.clone(),
            bus,
            gain: play.gain,
            pan: play.pan,
            pitch: play.pitch,
            region,
            priority: play.priority,
            fade_in: play.fade_in,
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

    /// A new bus feeding `parent`.
    pub fn add_bus(&self, name: &str, parent: &Bus) -> Result<Bus, Error> {
        let mut control = self.control();
        if control.buses.iter().any(|(bus, _)| bus == name) {
            return Err(Error::BusExists(name.into()));
        }
        if control.buses.len() >= MAX_BUSES {
            return Err(Error::TooManyBuses);
        }
        let index = control.buses.len() as u16;
        let command = Command::AddBus {
            bus: index,
            parent: parent.index,
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
            sounding: meter.sounding.load(Ordering::Relaxed),
            running: meter.running.load(Ordering::SeqCst),
            starts: meter.starts.load(Ordering::Relaxed),
            stops: meter.stops.load(Ordering::Relaxed),
            start_micros: meter.start_micros.load(Ordering::Relaxed),
            losses: meter.losses.load(Ordering::Relaxed),
            failures: meter.failures.load(Ordering::Relaxed),
            underruns: meter.underruns.load(Ordering::Relaxed),
            block_asked: meter.block_asked.load(Ordering::Relaxed),
            block_heard: meter.block_heard.load(Ordering::Relaxed),
            play_heard: meter.play_heard.load(Ordering::Acquire),
            play_asked: meter.play_asked.load(Ordering::Relaxed),
            play_mixed: crate::device::clock::nanos(meter.play_called.load(Ordering::Relaxed)),
            peak: f32::from_bits(meter.peak.load(Ordering::Relaxed)),
        }
    }
}

/// How a sound is played: the defaults are master, whole gain, centred,
/// its own pitch, now, once.
#[derive(Clone, Debug)]
pub struct Play {
    bus: Option<u16>,
    gain: f32,
    pan: f32,
    pitch: f32,
    at: Option<Time>,
    looping: bool,
    region: Option<Range<u64>>,
    priority: i8,
    fade_in: Option<Ramp>,
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
        }
    }
    /// The bus it plays into.
    pub fn bus(mut self, bus: &Bus) -> Play {
        self.bus = Some(bus.index);
        self
    }
    /// Its gain, 1 as recorded.
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
        self.send(Command::Voice {
            id: self.id,
            param,
            value,
            ramp,
        })
    }

    /// Moves its gain to `gain`.
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

    /// Moves its gain to `gain`: a fade, with a [`Ramp`]'s curve.
    pub fn set_gain(&self, gain: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        let ramp = ramp.into();
        self.audio.send(
            self.time,
            Command::BusGain {
                bus: self.index,
                value: gain,
                ramp,
            },
        )
    }

    /// Fades it to `gain` over `seconds` along `curve`.
    pub fn fade_to(&self, gain: f32, seconds: f64, curve: Curve) -> Result<(), Error> {
        self.set_gain(gain, Ramp::seconds(seconds).curve(curve))
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
        });
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
        let (audio, mut mixer) = Audio::offline(Config::default());
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
}
