//! The main thread's side: [`Audio`] and the handles it gives out. Each
//! call becomes a command stamped with a frame and goes on a fixed queue;
//! a full queue is an error, never a wait.
use crate::mixer::{Command, MAX_BUSES, Meter, Mixer, Return, Setup, VoiceParam};
use crate::queue::{Consumer, Producer, queue};
use crate::ramp::{Curve, Ramp};
use crate::resample::{Interpolation, Region};
use crate::sound::{Sound, Source};
use std::collections::HashSet;
use std::ops::{Add, Range, Sub};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

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
    /// A [`Config`] the engine can't be made with, and why.
    Config(String),
    /// A handle (a [`Bus`]) from another engine.
    Foreign,
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
            Error::Config(why) => write!(f, "an audio engine can't be made with {why}"),
            Error::Foreign => f.write_str("a bus from another audio engine"),
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
}

/// The most voices an engine can have.
pub const MAX_VOICES: usize = 4096;

impl Config {
    /// Whether an engine can be made with it: a rate from 1 kHz to
    /// 768 kHz, 1 to 64 channels, 1 to [`MAX_VOICES`] voices, 16 to 2^20
    /// commands, and a limiter's ceiling finite and above zero.
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
        }
    }
}

/// What the mixer has done, as of its last block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// Engines made so far: each takes the next as its identity, which its
/// handles carry.
static ENGINES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Inner {
    /// Which engine this is: a handle from another is refused.
    id: u64,
    control: Mutex<Control>,
    meter: Arc<Meter>,
    rate: u32,
    latency: usize,
}

/// The engine, as the main thread holds it. Cloning it is cheap; every
/// clone drives the same mixer.
#[derive(Clone)]
pub struct Audio(Arc<Inner>);

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("rate", &self.0.rate)
            .field("now", &self.now())
            .finish()
    }
}

/// The mixer run by hand, on a clock of its own: tests compare what it
/// makes sample for sample, and a video renders its soundtrack with it.
pub struct Offline {
    mixer: Mixer,
    channels: usize,
}

impl Offline {
    /// Mixes the next `out.len() / channels` frames into `out`.
    pub fn render(&mut self, out: &mut [f32]) {
        self.mixer.render(out, self.channels);
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
        Ok((
            audio,
            Offline {
                mixer,
                channels: config.channels,
            },
        ))
    }

    pub(crate) fn with_mixer(config: &Config) -> Result<(Audio, Mixer), Error> {
        config.check()?;
        let voices = config.voices;
        let (commands, from_main) = queue(config.commands);
        let (to_main, returns) = queue(voices * 2 + 64);
        let meter = Arc::new(Meter::default());
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
                returns,
                next_id: 1,
                buses,
                live: HashSet::new(),
            }),
            meter,
            rate: setup.rate,
            latency,
        }));
        for bus in 1..4 {
            audio
                .send(0, Command::AddBus { bus, parent: 0 })
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
        // Sounds the mixer is done with are dropped here, on this thread.
        while let Some(Return::Ended { id, sound }) = control.returns.pop() {
            control.live.remove(&id);
            drop(sound);
        }
        control
    }

    fn send(&self, time: u64, command: Command) -> Result<(), Error> {
        self.control()
            .commands
            .push((time, command))
            .map_err(|_| Error::QueueFull)
    }

    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.0.rate
    }

    /// The frame the mixer will mix next.
    pub fn now(&self) -> Time {
        Time(self.0.meter.now.load(Ordering::Acquire))
    }

    /// `seconds` as frames, to add to a [`Time`].
    pub fn seconds(&self, seconds: f64) -> u64 {
        (seconds.max(0.) * f64::from(self.0.rate)).round() as u64
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
        let bus = match play.bus {
            None => 0,
            Some((engine, bus)) if engine == self.0.id => bus,
            Some(_) => return Err(Error::Foreign),
        };
        let mut control = self.control();
        let id = control.next_id;
        let command = Command::Play {
            id,
            sound: sound.0.clone(),
            bus,
            gain: play.gain,
            pan: play.pan,
            pitch: play.pitch,
            region,
            priority: play.priority,
            fade_in: play
                .fade_in
                .map(|ramp| (ramp.frames(self.0.rate), ramp.curve)),
        };
        let time = play.at.map_or(0, |at| at.0);
        control
            .commands
            .push((time, command))
            .map_err(|_| Error::QueueFull)?;
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
        control
            .commands
            .push((
                0,
                Command::AddBus {
                    bus: index,
                    parent: parent.index,
                },
            ))
            .map_err(|_| Error::QueueFull)?;
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
    /// The bus it plays into: one of the engine's it's played on (another
    /// engine's is an error).
    pub fn bus(mut self, bus: &Bus) -> Play {
        self.bus = Some((bus.audio.0.id, bus.index));
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
            frames: fade.frames(self.audio.rate()),
            curve: fade.curve,
        })
    }

    fn set(&self, param: VoiceParam, value: f32, ramp: impl Into<Ramp>) -> Result<(), Error> {
        let ramp = ramp.into();
        self.send(Command::Voice {
            id: self.id,
            param,
            value,
            frames: ramp.frames(self.audio.rate()),
            curve: ramp.curve,
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
                frames: ramp.frames(self.audio.rate()),
                curve: ramp.curve,
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
