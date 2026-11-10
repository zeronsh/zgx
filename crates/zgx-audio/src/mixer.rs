//! The mixer: what runs on the audio thread. It applies each command at
//! its frame, mixes voices into their buses, buses into their parents, and
//! master through the limiter. It touches only memory made before it ran
//! and never locks, allocates, frees or calls the system: sounds it's done
//! with go back to the main thread to be dropped there.
use crate::limiter::Limiter;
use crate::queue::{Consumer, Producer};
use crate::ramp::{Curve, Ramp, Smoothed};
use crate::resample::{Interpolation, ONE, Position, Region, Resampler, Step};
use crate::ring::Reader;
use crate::sound::SoundData;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The most frames mixed at once; a device's longer blocks are mixed in
/// pieces of this.
pub(crate) const MAX_BLOCK: usize = 128;
/// Buses at most, master included.
pub(crate) const MAX_BUSES: usize = 32;
/// Slots past the voice limit, for voices fading out after being stolen.
const FADING_SLOTS: usize = 16;
/// Frames before a play that found no slot tries again.
const RETRY: u64 = 64;

/// What the main thread asks of the mixer.
pub(crate) enum Command {
    Play {
        id: u64,
        sound: Feed,
        bus: u16,
        gain: f32,
        pan: f32,
        pitch: f32,
        region: Region,
        priority: i8,
        /// A fade in; `None`, a click-free start.
        fade_in: Option<Ramp>,
    },
    // Ramps are in seconds, made frames by the mixer at its rate: a device
    // may set the rate after a command was sent.
    Stop {
        id: u64,
        ramp: Ramp,
    },
    Voice {
        id: u64,
        param: VoiceParam,
        value: f32,
        ramp: Ramp,
    },
    BusGain {
        bus: u16,
        value: f32,
        ramp: Ramp,
    },
    Mute {
        bus: u16,
        on: bool,
    },
    Solo {
        bus: u16,
        on: bool,
    },
    /// Silenced by the system (the user's interface-sound setting), apart
    /// from the app's own mute.
    Quiet {
        bus: u16,
        on: bool,
    },
    AddBus {
        bus: u16,
        parent: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceParam {
    Gain,
    Pan,
    Pitch,
}

/// What the mixer gives back.
pub(crate) enum Return {
    /// A voice finished (or never started); its sound is the main thread's
    /// to drop.
    Ended { id: u64, sound: Feed },
}

/// A drive's bits when nothing drives it: a NaN.
pub(crate) const UNDRIVEN: u32 = 0x7fc0_0000;

/// How long a bus takes to reach a value its drive was set to.
const GLIDE_SECONDS: f64 = 0.01;

/// Buckets of a microsecond for how long each block took to mix; the last
/// holds everything longer.
pub(crate) const TIME_BUCKETS: usize = 2048;

/// Counters the mixer publishes after each block and anyone reads.
pub(crate) struct Meter {
    pub now: AtomicU64,
    pub voices: AtomicU64,
    pub stolen: AtomicU64,
    pub dropped: AtomicU64,
    pub late: AtomicU64,
    pub blocks: AtomicU64,
    /// Frames a second, as the device last set them.
    pub rate: AtomicU64,
    /// Commands sent by the main thread, and taken off the queue by the
    /// mixer: a device isn't stopped while they differ.
    pub sent: AtomicU64,
    pub collected: AtomicU64,
    /// The last frame anything could be heard: a voice, a fade, the
    /// limiter's tail.
    pub audible: AtomicU64,
    /// Frames mixed with sound reaching master, before master's gain:
    /// proof a run at zero gain mixed something.
    pub sounding: AtomicU64,
    /// On the device's clock, in nanoseconds: when the last block was asked
    /// for, when its first frame will be heard, and when the most recent
    /// play's first frame will be.
    pub block_asked: AtomicU64,
    pub block_heard: AtomicU64,
    pub play_heard: AtomicU64,
    /// The device's clock when the block that started the most recent play
    /// was asked for.
    pub play_asked: AtomicU64,
    /// The host clock (macOS) when that block was rendered.
    pub play_called: AtomicU64,
    /// The loudest sample sent to the output since the engine was made (an
    /// f32's bits: a positive float's order is its bits').
    pub peak: AtomicU32,
    /// How long blocks took to mix, a bucket a microsecond.
    pub times: [AtomicU32; TIME_BUCKETS],
    /// The host's doings: streams started and stopped, how long the last
    /// start took, streams lost and rebuilt, failures to start, underruns
    /// the system reported, whether a stream runs now.
    pub starts: AtomicU64,
    pub stops: AtomicU64,
    pub start_micros: AtomicU64,
    pub losses: AtomicU64,
    pub failures: AtomicU64,
    pub underruns: AtomicU64,
    /// Times a stream ran dry.
    pub dry: AtomicU64,
    pub running: AtomicBool,
    /// Each bus's gain as the main thread drives it, an f32's bits
    /// ([`UNDRIVEN`] until driven): read by the mixer each block, so a
    /// slider or an animation moves it with no command at all.
    pub drives: [AtomicU32; MAX_BUSES],
    /// Voices given back, so far: the host tells the app when it moves.
    pub ended: AtomicU64,
    /// Called by the host when voices end: the app's waker.
    pub notify: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// Why the device last failed. The host and the main thread take it;
    /// the audio thread never does.
    pub error: Mutex<Option<String>>,
}

impl Meter {
    pub fn new(rate: u32) -> Meter {
        let zero = || AtomicU64::new(0);
        Meter {
            now: zero(),
            voices: zero(),
            stolen: zero(),
            dropped: zero(),
            late: zero(),
            blocks: zero(),
            rate: AtomicU64::new(u64::from(rate)),
            sent: zero(),
            collected: zero(),
            audible: zero(),
            sounding: zero(),
            block_asked: zero(),
            block_heard: zero(),
            play_heard: zero(),
            play_asked: zero(),
            play_called: zero(),
            peak: AtomicU32::new(0),
            times: std::array::from_fn(|_| AtomicU32::new(0)),
            starts: zero(),
            stops: zero(),
            start_micros: zero(),
            losses: zero(),
            failures: zero(),
            underruns: zero(),
            dry: zero(),
            running: AtomicBool::new(false),
            drives: std::array::from_fn(|_| AtomicU32::new(UNDRIVEN)),
            ended: zero(),
            notify: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    /// Whether commands were sent that the mixer hasn't taken yet.
    pub fn pending(&self) -> bool {
        self.sent.load(Ordering::SeqCst) != self.collected.load(Ordering::SeqCst)
    }

    /// Tells the app, if it asked to be told, that voices were given back
    /// (so a `playing` state can change). Called by the host, never the
    /// audio thread.
    pub fn notify(&self) {
        if let Some(notify) = &*self
            .notify
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            notify();
        }
    }

    pub fn set_error(&self, why: String) {
        *self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(why);
    }
}

/// When a device asks for a block, and when its first frame will be heard,
/// in nanoseconds of the device's own clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timing {
    pub asked: u64,
    pub heard: u64,
}

/// A command and the frame it's for (0: as soon as it can be).
struct Scheduled {
    time: u64,
    seq: u64,
    command: Command,
}
impl PartialEq for Scheduled {
    fn eq(&self, other: &Self) -> bool {
        (self.time, self.seq) == (other.time, other.seq)
    }
}
impl Eq for Scheduled {}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Scheduled {
    /// Earliest first, then in the order sent (the heap pops the greatest).
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (other.time, other.seq).cmp(&(self.time, self.seq))
    }
}

struct Voice {
    /// 0: a free slot.
    id: u64,
    sound: Option<Feed>,
    bus: u16,
    at: Position,
    /// The step at pitch 1: the sound's rate over the output's.
    base_step: f64,
    region: Region,
    gain: Smoothed,
    pan: Smoothed,
    pitch: Smoothed,
    /// Fades in and out: starts, stops, steals.
    envelope: Smoothed,
    stopping: bool,
    /// Done; its sound waits for room on the return queue.
    ended: bool,
    /// Not mixed yet: a stop now ends it without a fade.
    fresh: bool,
    priority: i8,
    resampler: Resampler,
}

impl Voice {
    fn free() -> Voice {
        Voice {
            id: 0,
            sound: None,
            bus: 0,
            at: 0,
            base_step: 1.,
            region: Region {
                start: 0,
                end: 0,
                looping: false,
            },
            gain: Smoothed::new(1.),
            pan: Smoothed::new(0.),
            pitch: Smoothed::new(1.),
            envelope: Smoothed::new(1.),
            stopping: false,
            ended: false,
            fresh: false,
            priority: 0,
            resampler: Resampler::new(Interpolation::Linear),
        }
    }

    fn live(&self) -> bool {
        self.id != 0 && !self.ended && !self.stopping
    }

    fn loudness(&self) -> f32 {
        self.gain.value() * self.envelope.value()
    }

    fn step(&self, pitch: f32) -> u64 {
        ((self.base_step * f64::from(pitch) * ONE as f64) as u64).max(1)
    }

    /// Mixes the next `n` frames into `left` and `right`. Returns whether
    /// it still sounds.
    fn mix(
        &mut self,
        n: usize,
        scratch: &mut Scratch,
        left: &mut [f32],
        right: &mut [f32],
        click: u32,
        underruns: &mut u64,
    ) -> bool {
        self.fresh = false;
        let fixed = self.pitch.settled();
        let step = self.step(self.pitch.value());
        if !fixed {
            let pitches = &mut scratch.values[..n];
            self.pitch.fill(pitches);
            let scale = self.base_step * ONE as f64;
            for (step, &pitch) in scratch.steps[..n].iter_mut().zip(pitches.iter()) {
                *step = ((scale * f64::from(pitch)) as u64).max(1);
            }
        }
        if let Some(Feed::Stream(stream)) = &mut self.sound {
            let advance: u64 = if fixed {
                step * n as u64
            } else {
                scratch.steps[..n].iter().sum()
            };
            let need = ((self.at + advance) >> 32) as usize + 4;
            if !stream.ready(need, &mut self.envelope, click, underruns) {
                // Waiting for its first frames, or run dry: time passes,
                // nothing plays, the position holds.
                self.gain.advance(n);
                self.pan.advance(n);
                self.envelope.advance(n);
                stream.level.advance(n);
                return !(self.stopping && self.envelope.settled() && self.envelope.value() == 0.);
            }
        }
        let (sound, region, level) = match &mut self.sound {
            Some(Feed::Sound(sound)) => (&**sound, self.region, None),
            Some(Feed::Stream(stream)) => {
                let region = Region {
                    start: 0,
                    end: stream.filled as u64,
                    looping: false,
                };
                (&stream.window, region, Some(&mut stream.level))
            }
            None => unreachable!("a playing voice has its sound"),
        };
        // Two calls, each with its kind of step known, so each is made for
        // it: one call taking either made the cubic 10% dearer.
        let got = if fixed {
            self.resampler.render(
                sound,
                region,
                &mut self.at,
                Step::Fixed(step),
                &mut scratch.left[..n],
                &mut scratch.right[..n],
            )
        } else {
            self.resampler.render(
                sound,
                region,
                &mut self.at,
                Step::PerFrame(&scratch.steps[..n]),
                &mut scratch.left[..n],
                &mut scratch.right[..n],
            )
        };
        let source_right: &[f32] = if sound.channels == 1 {
            &scratch.left
        } else {
            &scratch.right
        };
        let source_left = &scratch.left;
        let level_settled = level.as_ref().is_none_or(|l| l.settled());
        if self.gain.settled() && self.envelope.settled() && self.pan.settled() && level_settled {
            let gain = self.gain.value() * self.envelope.value() * level.map_or(1., |l| l.value());
            if gain != 0. {
                let (pl, pr) = pan_gains(self.pan.value());
                let (gl, gr) = (gain * pl, gain * pr);
                for i in 0..got {
                    left[i] += source_left[i] * gl;
                    right[i] += source_right[i] * gr;
                }
            }
        } else {
            // Each moving parameter is filled for the block in a loop of
            // its own, which the compiler turns into vector instructions.
            let gains = &mut scratch.gains[..n];
            self.gain.fill(gains);
            for ramp in [Some(&mut self.envelope), level].into_iter().flatten() {
                if !ramp.settled() {
                    let values = &mut scratch.values[..n];
                    ramp.fill(values);
                    for (g, v) in gains.iter_mut().zip(values.iter()) {
                        *g *= v;
                    }
                } else if ramp.value() != 1. {
                    let value = ramp.value();
                    gains.iter_mut().for_each(|g| *g *= value);
                }
            }
            if self.pan.settled() {
                let (pl, pr) = pan_gains(self.pan.value());
                for i in 0..got {
                    left[i] += source_left[i] * gains[i] * pl;
                    right[i] += source_right[i] * gains[i] * pr;
                }
            } else {
                let pans = &mut scratch.values[..n];
                self.pan.fill(pans);
                for i in 0..got {
                    let (pl, pr) = pan_gains(pans[i]);
                    left[i] += source_left[i] * gains[i] * pl;
                    right[i] += source_right[i] * gains[i] * pr;
                }
            }
        }
        if let Some(Feed::Stream(stream)) = &mut self.sound {
            stream.shift(&mut self.at);
        }
        let faded = self.stopping && self.envelope.settled() && self.envelope.value() == 0.;
        got == n && !faded
    }
}

/// What a voice plays: a sound in memory, or a stream decoded as it plays.
pub(crate) enum Feed {
    Sound(Arc<SoundData>),
    Stream(Box<Streamed>),
}

impl Feed {
    fn data(&self) -> &SoundData {
        match self {
            Feed::Sound(sound) => sound,
            Feed::Stream(stream) => &stream.window,
        }
    }
}

/// Frames a stream's window holds: a block's worth at the highest pitch
/// and rate, and some.
const WINDOW: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    /// Not started: waiting for its first frames.
    Waiting,
    Flowing,
    /// Ran dry: silent until frames come again.
    Dry,
}

/// A stream as a voice holds it: frames from the decode thread's ring,
/// pulled into a window the resampler reads like a sound of its own.
pub(crate) struct Streamed {
    reader: Reader,
    window: SoundData,
    /// Frames in the window.
    filled: usize,
    flow: Flow,
    /// Fades out as the ring runs low, and back in when it fills.
    level: Smoothed,
    /// Frames of that fade (5 ms), at the stream's rate.
    reserve: usize,
}

impl Streamed {
    /// Allocates: made on the main thread.
    pub fn new(reader: Reader, channels: usize, rate: u32) -> Streamed {
        Streamed {
            reader,
            window: SoundData {
                samples: vec![0.; WINDOW * channels].into(),
                channels,
                rate,
            },
            filled: 0,
            flow: Flow::Waiting,
            level: Smoothed::new(1.),
            reserve: (rate / 200).max(1) as usize,
        }
    }

    /// Frames it holds: in the window and waiting in the ring.
    fn held(&self) -> usize {
        self.filled + self.reader.ready() / self.window.channels
    }

    /// Pulls frames until the window holds `need` (or is full), and says
    /// whether to play this block: not while waiting for its first frames,
    /// nor dry. Starts the fade out as frames run low, the fade in as they
    /// come back, and counts each time it runs dry.
    fn ready(
        &mut self,
        need: usize,
        envelope: &mut Smoothed,
        click: u32,
        underruns: &mut u64,
    ) -> bool {
        let channels = self.window.channels;
        let want = need.min(WINDOW).saturating_sub(self.filled) * channels;
        if want > 0 {
            let ready = self.reader.ready() / channels * channels;
            let at = self.filled * channels;
            let got = self
                .reader
                .read(&mut self.window.samples[at..at + want.min(ready)]);
            self.filled += got / channels;
        }
        // Nothing more is coming: what's held plays out to the end, with no
        // fade for running low.
        let ended = self.reader.finished();
        let (held, reserve) = (self.held(), self.reserve);
        match self.flow {
            Flow::Waiting if ended || held >= need + reserve => {
                self.flow = Flow::Flowing;
                // A stream starting away from zero fades in over its first
                // millisecond, as a sound does.
                let first = self.window.samples[..channels]
                    .iter()
                    .fold(0f32, |p, s| p.max(s.abs()));
                if first > 1e-3 && envelope.settled() && envelope.value() == 1. {
                    *envelope = Smoothed::new(0.);
                    envelope.set(1., click, Curve::Linear);
                }
                true
            }
            Flow::Waiting => false,
            Flow::Flowing if !ended && self.filled < need => {
                // Out of frames before the fade could finish.
                self.flow = Flow::Dry;
                self.level = Smoothed::new(0.);
                *underruns += 1;
                false
            }
            Flow::Flowing => {
                if !ended && held < need + reserve && self.level.target() == 1. {
                    self.level.set(0., reserve as u32, Curve::Linear);
                } else if (ended || held >= need + 4 * reserve) && self.level.target() == 0. {
                    self.level.set(1., reserve as u32, Curve::Linear);
                }
                if self.level.settled() && self.level.value() == 0. {
                    self.flow = Flow::Dry;
                    *underruns += 1;
                    return false;
                }
                true
            }
            Flow::Dry if ended || held >= need + 4 * reserve => {
                self.flow = Flow::Flowing;
                self.level.set(1., reserve as u32, Curve::Linear);
                self.filled >= need || ended
            }
            Flow::Dry => false,
        }
    }

    /// Drops the frames the position has passed, keeping a few before it
    /// for the interpolator.
    fn shift(&mut self, at: &mut Position) {
        let passed = ((*at >> 32) as usize).saturating_sub(4).min(self.filled);
        if passed > 0 {
            let channels = self.window.channels;
            self.window
                .samples
                .copy_within(passed * channels..self.filled * channels, 0);
            self.filled -= passed;
            *at -= (passed as u64) << 32;
        }
    }
}

/// Left and right gains for a pan from -1 (left) to 1 (right): a balance,
/// where the near side stays whole and the far one falls along an
/// equal-power curve (the square root of its share), so a sound in the
/// middle plays as it was made. A square root rounds the same everywhere,
/// and costs a few cycles where a sine and cosine cost dozens.
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let right = (pan.clamp(-1., 1.) + 1.) * 0.5;
    (
        ((1. - right) * 2.).sqrt().min(1.),
        (right * 2.).sqrt().min(1.),
    )
}

struct Bus {
    used: bool,
    parent: u16,
    gain: Smoothed,
    /// 0 while muted, or silenced by another bus's solo.
    gate: Smoothed,
    muted: bool,
    soloed: bool,
    /// Silenced by the system's setting.
    quiet: bool,
    /// The drive's bits last applied (NaN's: not driven).
    driven: u32,
    left: Box<[f32; MAX_BLOCK]>,
    right: Box<[f32; MAX_BLOCK]>,
    /// Holds this block's sound (else its buffers are stale).
    touched: bool,
}

impl Bus {
    fn new() -> Bus {
        Bus {
            used: false,
            parent: 0,
            gain: Smoothed::new(1.),
            gate: Smoothed::new(1.),
            muted: false,
            soloed: false,
            quiet: false,
            driven: UNDRIVEN,
            left: Box::new([0.; MAX_BLOCK]),
            right: Box::new([0.; MAX_BLOCK]),
            touched: false,
        }
    }

    fn touch(&mut self, n: usize) {
        if !self.touched {
            self.left[..n].fill(0.);
            self.right[..n].fill(0.);
            self.touched = true;
        }
    }
}

struct Scratch {
    left: [f32; MAX_BLOCK],
    right: [f32; MAX_BLOCK],
    gains: [f32; MAX_BLOCK],
    /// A moving parameter's value for each frame of the block.
    values: [f32; MAX_BLOCK],
    steps: [u64; MAX_BLOCK],
}

/// What a mixer is made with.
pub(crate) struct Setup {
    pub rate: u32,
    pub voices: usize,
    pub commands: usize,
    pub limiter: Option<f32>,
    pub interpolation: Interpolation,
}

pub(crate) struct Mixer {
    rate: u32,
    interpolation: Interpolation,
    commands: Consumer<(u64, Command)>,
    returns: Producer<Return>,
    meter: Arc<Meter>,
    schedule: BinaryHeap<Scheduled>,
    seq: u64,
    voices: Box<[Voice]>,
    max_live: usize,
    buses: Box<[Bus]>,
    bus_count: usize,
    scratch: Box<Scratch>,
    limiter: Option<Limiter>,
    now: u64,
    stolen: u64,
    dropped: u64,
    late: u64,
    blocks: u64,
    collected: u64,
    sounding: u64,
    /// Voices given back.
    ended: u64,
    /// Times a stream ran dry.
    underruns: u64,
    /// The frame the last play was applied on, this block.
    started: Option<u64>,
    /// Frames of the shortest de-click fade (1 ms), and of a steal's or a
    /// mute's (5 ms).
    click: u32,
    declick: u32,
}

impl Mixer {
    pub fn new(
        setup: &Setup,
        commands: Consumer<(u64, Command)>,
        returns: Producer<Return>,
        meter: Arc<Meter>,
    ) -> Mixer {
        let mut buses: Box<[Bus]> = (0..MAX_BUSES).map(|_| Bus::new()).collect();
        buses[0].used = true;
        Mixer {
            rate: setup.rate,
            interpolation: setup.interpolation,
            commands,
            returns,
            meter,
            schedule: BinaryHeap::with_capacity(setup.commands),
            seq: 0,
            voices: (0..setup.voices + FADING_SLOTS)
                .map(|_| Voice::free())
                .collect(),
            max_live: setup.voices,
            buses,
            bus_count: 1,
            scratch: Box::new(Scratch {
                left: [0.; MAX_BLOCK],
                right: [0.; MAX_BLOCK],
                gains: [0.; MAX_BLOCK],
                values: [0.; MAX_BLOCK],
                steps: [0; MAX_BLOCK],
            }),
            limiter: setup
                .limiter
                .map(|ceiling| Limiter::new(setup.rate, ceiling)),
            now: 0,
            stolen: 0,
            dropped: 0,
            late: 0,
            blocks: 0,
            collected: 0,
            sounding: 0,
            ended: 0,
            underruns: 0,
            started: None,
            click: (setup.rate / 1000).max(1),
            declick: (setup.rate / 200).max(1),
        }
    }

    fn frames(&self, seconds: f64) -> u32 {
        (seconds * f64::from(self.rate)).round() as u32
    }

    /// Moves each bus whose drive changed toward its new value over
    /// `frames` (at once while nothing plays).
    fn follow_drives(&mut self, frames: u32) {
        for (bus, drive) in self.buses[..self.bus_count]
            .iter_mut()
            .zip(&self.meter.drives)
        {
            let bits = drive.load(Ordering::Relaxed);
            if bits != bus.driven {
                bus.driven = bits;
                let value = f32::from_bits(bits);
                if !value.is_nan() {
                    bus.gain.set(value.max(0.), frames, Curve::Linear);
                }
            }
        }
    }

    /// Applies what's been sent while no device runs (off the audio
    /// thread): gains, mutes and solos take effect, plays find their
    /// voices, and the clock stands still. Returns whether anything would
    /// sound: a voice, or a command due later.
    pub fn idle(&mut self) -> bool {
        self.collect();
        self.follow_drives(0);
        self.apply_due();
        self.meter.collected.store(self.collected, Ordering::SeqCst);
        self.voices.iter().any(|v| v.id != 0) || !self.schedule.is_empty()
    }

    /// Mixes at `rate` from now on: a device whose rate isn't the one the
    /// mixer was made with. Off the audio thread: it allocates.
    pub fn set_rate(&mut self, rate: u32) {
        if rate == self.rate || rate == 0 {
            return;
        }
        self.rate = rate;
        self.click = (rate / 1000).max(1);
        self.declick = (rate / 200).max(1);
        if let Some(limiter) = &self.limiter {
            self.limiter = Some(Limiter::new(rate, limiter.ceiling()));
        }
        for voice in self.voices.iter_mut() {
            if let Some(feed) = &voice.sound {
                voice.base_step = f64::from(feed.data().rate) / f64::from(rate);
            }
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames the output lags a command's time: the limiter's look-ahead.
    pub fn latency(&self) -> usize {
        self.limiter.as_ref().map_or(0, Limiter::latency)
    }

    /// Mixes the next `out.len() / channels` frames into `out`, interleaved.
    pub fn render(&mut self, out: &mut [f32], channels: usize, timing: Option<Timing>) {
        let frames = out.len() / channels;
        let first = self.now;
        let mut done = 0;
        let mut peak = 0u32;
        self.started = None;
        self.collect();
        self.follow_drives(self.frames(GLIDE_SECONDS));
        while done < frames {
            self.apply_due();
            let mut n = (frames - done).min(MAX_BLOCK);
            if let Some(next) = self.schedule.peek() {
                n = n.min(next.time.saturating_sub(self.now).max(1) as usize);
            }
            self.mix(n);
            let master = &self.buses[0];
            if master.touched {
                // As bits without their sign: a float's size orders as its
                // bits do, and an integer maximum vectorises.
                let loudest = |side: &[f32]| {
                    side.iter()
                        .fold(0u32, |p, s| p.max(s.to_bits() & 0x7fff_ffff))
                };
                peak = peak
                    .max(loudest(&master.left[..n]))
                    .max(loudest(&master.right[..n]));
            }
            let out = &mut out[done * channels..(done + n) * channels];
            match channels {
                1 => {
                    for (o, (l, r)) in out
                        .iter_mut()
                        .zip(master.left.iter().zip(master.right.iter()))
                    {
                        *o = (l + r) * 0.5;
                    }
                }
                2 => {
                    for (o, (l, r)) in out
                        .as_chunks_mut::<2>()
                        .0
                        .iter_mut()
                        .zip(master.left.iter().zip(master.right.iter()))
                    {
                        *o = [*l, *r];
                    }
                }
                _ => {
                    for (o, (l, r)) in out
                        .chunks_exact_mut(channels)
                        .zip(master.left.iter().zip(master.right.iter()))
                    {
                        (o[0], o[1]) = (*l, *r);
                        o[2..].fill(0.);
                    }
                }
            }
            self.now += n as u64;
            done += n;
        }
        self.blocks += 1;
        let live = self.voices.iter().filter(|v| v.id != 0 && !v.ended).count();
        let meter = &self.meter;
        // Anything playing, fading, in the limiter's tail or due to start
        // counts as heard: the device isn't stopped under it.
        let quiet = self.limiter.as_ref().is_none_or(Limiter::silent);
        if self.voices.iter().any(|v| v.id != 0) || !quiet || !self.schedule.is_empty() {
            meter.audible.store(self.now, Ordering::Relaxed);
        }
        if let Some(timing) = timing {
            let after = |frames: u64| timing.heard + frames * 1_000_000_000 / u64::from(self.rate);
            meter.block_asked.store(timing.asked, Ordering::Relaxed);
            meter.block_heard.store(timing.heard, Ordering::Relaxed);
            if let Some(started) = self.started {
                let latency = self.limiter.as_ref().map_or(0, Limiter::latency) as u64;
                meter.play_asked.store(timing.asked, Ordering::Relaxed);
                meter
                    .play_heard
                    .store(after(started - first + latency), Ordering::Release);
            }
        }
        meter.collected.store(self.collected, Ordering::SeqCst);
        meter.ended.store(self.ended, Ordering::Release);
        meter.sounding.store(self.sounding, Ordering::Relaxed);
        meter.dry.store(self.underruns, Ordering::Relaxed);
        meter.peak.fetch_max(peak, Ordering::Relaxed);
        meter.voices.store(live as u64, Ordering::Relaxed);
        meter.stolen.store(self.stolen, Ordering::Relaxed);
        meter.dropped.store(self.dropped, Ordering::Relaxed);
        meter.late.store(self.late, Ordering::Relaxed);
        meter.blocks.store(self.blocks, Ordering::Relaxed);
        meter.now.store(self.now, Ordering::Release);
    }

    /// Moves commands from the queue into the schedule, as many as fit.
    fn collect(&mut self) {
        while self.schedule.len() < self.schedule.capacity() {
            let Some((time, command)) = self.commands.pop() else {
                break;
            };
            self.seq += 1;
            self.collected += 1;
            self.schedule.push(Scheduled {
                time,
                seq: self.seq,
                command,
            });
        }
    }

    fn apply_due(&mut self) {
        while self
            .schedule
            .peek()
            .is_some_and(|next| next.time <= self.now)
        {
            let Scheduled { time, seq, command } = self.schedule.pop().expect("peeked");
            // A voice's other commands carry its start: only plays are late.
            if time != 0 && time < self.now && matches!(command, Command::Play { .. }) {
                self.late += 1;
            }
            if let Err(command) = self.apply(command) {
                // No slot yet: tried again a little later. The heap has
                // room: this one just left it.
                self.schedule.push(Scheduled {
                    time: self.now + RETRY,
                    seq,
                    command,
                });
            }
        }
    }

    fn voice(&mut self, id: u64) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id && !v.ended)
    }

    fn apply(&mut self, command: Command) -> Result<(), Command> {
        match command {
            Command::Play { .. } => return self.play(command),
            Command::Stop { id, ramp } => {
                let (click, frames, curve) = (self.click, ramp.frames(self.rate), ramp.curve);
                if let Some(voice) = self.voice(id) {
                    if voice.fresh {
                        // Stopped before a frame was heard: no fade to make.
                        voice.ended = true;
                    } else {
                        voice.envelope.set(0., frames.max(click), curve);
                        voice.stopping = true;
                    }
                }
            }
            Command::Voice {
                id,
                param,
                value,
                ramp,
            } => {
                let (frames, curve) = (ramp.frames(self.rate), ramp.curve);
                if let Some(voice) = self.voice(id) {
                    match param {
                        VoiceParam::Gain => voice.gain.set(value.max(0.), frames, curve),
                        VoiceParam::Pan => {
                            voice.pan.set(value.clamp(-1., 1.), frames, Curve::Linear)
                        }
                        VoiceParam::Pitch => {
                            voice.pitch.set(value.clamp(1. / 64., 16.), frames, curve)
                        }
                    }
                }
            }
            Command::BusGain { bus, value, ramp } => {
                let frames = ramp.frames(self.rate);
                self.buses[usize::from(bus)]
                    .gain
                    .set(value.max(0.), frames, ramp.curve);
            }
            Command::Mute { bus, on } => {
                self.buses[usize::from(bus)].muted = on;
                self.gate();
            }
            Command::Solo { bus, on } => {
                self.buses[usize::from(bus)].soloed = on;
                self.gate();
            }
            Command::Quiet { bus, on } => {
                self.buses[usize::from(bus)].quiet = on;
                self.gate();
            }
            Command::AddBus { bus, parent } => {
                let index = usize::from(bus);
                let new = &mut self.buses[index];
                new.used = true;
                new.parent = parent;
                self.bus_count = self.bus_count.max(index + 1);
                self.gate();
            }
        }
        Ok(())
    }

    fn play(&mut self, command: Command) -> Result<(), Command> {
        let Command::Play {
            id,
            ref sound,
            bus,
            gain,
            pan,
            pitch,
            region,
            priority,
            fade_in,
        } = command
        else {
            unreachable!("only plays come here");
        };
        if self.voices.iter().filter(|v| v.live()).count() >= self.max_live {
            // The quietest of the lowest priority, the new voice included, goes.
            let victim = self
                .voices
                .iter_mut()
                .filter(|v| v.live())
                .min_by(|a, b| {
                    (a.priority, a.loudness())
                        .partial_cmp(&(b.priority, b.loudness()))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("the pool is full of live voices");
            if (priority, gain) < (victim.priority, victim.loudness()) {
                let Command::Play { sound, .. } = command else {
                    unreachable!()
                };
                return match self.returns.push(Return::Ended { id, sound }) {
                    Ok(()) => {
                        self.dropped += 1;
                        Ok(())
                    }
                    Err(Return::Ended { id, sound }) => Err(Command::Play {
                        id,
                        sound,
                        bus,
                        gain,
                        pan,
                        pitch,
                        region,
                        priority,
                        fade_in,
                    }),
                };
            }
            victim.envelope.set(0., self.declick, Curve::Linear);
            victim.stopping = true;
            self.stolen += 1;
        }
        let (rate, click, interpolation) = (self.rate, self.click, self.interpolation);
        let Some(voice) = self.voices.iter_mut().find(|v| v.id == 0) else {
            return Err(command);
        };
        // A stream's first frames aren't here yet: it looks when they are.
        let start = match sound {
            Feed::Sound(data) => data.samples[..data.channels.min(data.samples.len())]
                .iter()
                .fold(0f32, |p, s| p.max(s.abs())),
            Feed::Stream(_) => 0.,
        };
        let source_rate = sound.data().rate;
        *voice = Voice {
            id,
            sound: None,
            bus,
            at: 0,
            base_step: f64::from(source_rate) / f64::from(rate),
            region,
            gain: Smoothed::new(gain.max(0.)),
            pan: Smoothed::new(pan.clamp(-1., 1.)),
            pitch: Smoothed::new(pitch.clamp(1. / 64., 16.)),
            envelope: Smoothed::new(1.),
            stopping: false,
            ended: false,
            fresh: true,
            priority,
            resampler: Resampler::new(interpolation),
        };
        match fade_in {
            Some(ramp) => {
                voice.envelope = Smoothed::new(0.);
                voice.envelope.set(1., ramp.frames(rate), ramp.curve);
            }
            // A sound that starts away from zero gets a millisecond's fade.
            None if start > 1e-3 => {
                voice.envelope = Smoothed::new(0.);
                voice.envelope.set(1., click, Curve::Linear);
            }
            None => {}
        }
        let Command::Play { sound, .. } = command else {
            unreachable!()
        };
        voice.sound = Some(sound);
        self.started = Some(self.now);
        Ok(())
    }

    /// Opens or closes each bus's gate for the mutes and solos now set: a
    /// bus plays when it isn't muted and, if any bus is soloed, it is,
    /// or it's on the way to or from one that is.
    fn gate(&mut self) {
        let count = self.bus_count;
        let mut on_path = [false; MAX_BUSES];
        let mut any_solo = false;
        for b in 0..count {
            if self.buses[b].used && self.buses[b].soloed {
                any_solo = true;
                let mut at = b;
                loop {
                    on_path[at] = true;
                    if at == 0 {
                        break;
                    }
                    at = usize::from(self.buses[at].parent);
                }
            }
        }
        let declick = self.declick;
        for (b, &on_path) in on_path.iter().enumerate().take(count) {
            let mut audible = !any_solo || on_path;
            if !audible {
                let mut at = b;
                while at != 0 {
                    at = usize::from(self.buses[at].parent);
                    if self.buses[at].soloed {
                        audible = true;
                        break;
                    }
                }
            }
            let bus = &mut self.buses[b];
            let target = if bus.muted || bus.quiet || !audible {
                0.
            } else {
                1.
            };
            if bus.gate.target() != target {
                bus.gate.set(target, declick, Curve::Linear);
            }
        }
    }

    fn mix(&mut self, n: usize) {
        for bus in &mut self.buses[..self.bus_count] {
            bus.touched = false;
        }
        for voice in self.voices.iter_mut() {
            if voice.id == 0 {
                continue;
            }
            if !voice.ended {
                let bus = &mut self.buses[usize::from(voice.bus)];
                bus.touch(n);
                if !voice.mix(
                    n,
                    &mut self.scratch,
                    &mut bus.left[..n],
                    &mut bus.right[..n],
                    self.click,
                    &mut self.underruns,
                ) {
                    voice.ended = true;
                }
            }
            if voice.ended {
                let sound = voice
                    .sound
                    .take()
                    .expect("an ended voice still holds its sound");
                match self.returns.push(Return::Ended {
                    id: voice.id,
                    sound,
                }) {
                    Ok(()) => {
                        voice.id = 0;
                        self.ended += 1;
                    }
                    Err(Return::Ended { sound, .. }) => voice.sound = Some(sound),
                }
            }
        }
        for b in (1..self.bus_count).rev() {
            let (below, above) = self.buses.split_at_mut(b);
            let bus = &mut above[0];
            if !bus.used {
                continue;
            }
            if !bus.touched {
                bus.gain.advance(n);
                bus.gate.advance(n);
                continue;
            }
            let parent = &mut below[usize::from(bus.parent)];
            parent.touch(n);
            sum_into(
                bus,
                n,
                &mut self.scratch.gains,
                &mut parent.left[..n],
                &mut parent.right[..n],
            );
        }
        let master = &mut self.buses[0];
        if master.touched
            && master.left[..n]
                .iter()
                .chain(&master.right[..n])
                .any(|&s| s != 0.)
        {
            self.sounding += n as u64;
        }
        if master.touched {
            if master.gain.settled() && master.gate.settled() {
                let gain = master.gain.value() * master.gate.value();
                if gain != 1. {
                    master.left[..n].iter_mut().for_each(|s| *s *= gain);
                    master.right[..n].iter_mut().for_each(|s| *s *= gain);
                }
            } else {
                let gains = &mut self.scratch.gains;
                gain_of(master, n, gains);
                for ((l, r), g) in master
                    .left
                    .iter_mut()
                    .zip(master.right.iter_mut())
                    .zip(&gains[..n])
                {
                    *l *= g;
                    *r *= g;
                }
            }
        } else {
            master.gain.advance(n);
            master.gate.advance(n);
            master.left[..n].fill(0.);
            master.right[..n].fill(0.);
        }
        if let Some(limiter) = &mut self.limiter {
            limiter.process(&mut master.left[..n], &mut master.right[..n]);
        }
    }
}

/// The bus's gain times its gate for each of the next `n` frames.
fn gain_of(bus: &mut Bus, n: usize, gains: &mut [f32; MAX_BLOCK]) {
    let gains = &mut gains[..n];
    bus.gain.fill(gains);
    if bus.gate.settled() {
        let gate = bus.gate.value();
        gains.iter_mut().for_each(|g| *g *= gate);
    } else {
        for g in gains.iter_mut() {
            *g *= bus.gate.next();
        }
    }
}

/// Adds `bus`'s block, after its gain, into its parent's.
fn sum_into(
    bus: &mut Bus,
    n: usize,
    gains: &mut [f32; MAX_BLOCK],
    left: &mut [f32],
    right: &mut [f32],
) {
    if bus.gain.settled() && bus.gate.settled() {
        let gain = bus.gain.value() * bus.gate.value();
        if gain != 0. {
            for i in 0..n {
                left[i] += bus.left[i] * gain;
                right[i] += bus.right[i] * gain;
            }
        }
        return;
    }
    gain_of(bus, n, gains);
    for i in 0..n {
        left[i] += bus.left[i] * gains[i];
        right[i] += bus.right[i] * gains[i];
    }
}
