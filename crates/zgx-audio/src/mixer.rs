//! The mixer: what runs on the audio thread. It applies each command at
//! its frame, mixes voices into their buses, buses into their parents, and
//! master through the limiter. It touches only memory made before it ran
//! and never locks, allocates, frees or calls the system: sounds it's done
//! with go back to the main thread to be dropped there.
use crate::effect::Slot;
use crate::effects::VoiceLowPass;
use crate::limiter::Limiter;
use crate::queue::{Consumer, Producer};
use crate::ramp::{Curve, Ramp, Smoothed};
use crate::resample::{Interpolation, ONE, Position, Region, Resampler, Step};
use crate::ring::Reader;
use crate::sound::SoundData;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The most frames mixed at once; a device's longer blocks are mixed in
/// pieces of this.
pub(crate) const MAX_BLOCK: usize = 128;
/// Buses at most, master included.
pub(crate) const MAX_BUSES: usize = 32;
/// Effects a bus can hold.
pub(crate) const MAX_EFFECTS: usize = 8;
/// The most a sample can be as it leaves a bus's effects, either way: 2^18.
/// With every voice and every bus's effects bounded, the loudest sum
/// through the deepest tree of buses is under 2^120: finite.
const EFFECT_LIMIT: f32 = 262_144.;
/// Frames of the clock between the points where a bus decides whether
/// its effects run on silence: decided there, and only there, what an
/// effect runs on doesn't depend on how the device splits its blocks.
pub(crate) const CELL: u64 = MAX_BLOCK as u64;
/// Slots past the voice limit, for voices fading out after being stolen.
pub(crate) const FADING_SLOTS: usize = 16;
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
        law: PanLaw,
        /// Its own low-pass's cutoff, in hertz.
        cutoff: f32,
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
    /// An effect, prepared, for the end of a bus's chain.
    AddEffect {
        bus: u16,
        slot: Box<Slot>,
    },
    RemoveEffect {
        id: u64,
    },
    EffectParam {
        id: u64,
        index: u16,
        value: f32,
        ramp: Ramp,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VoiceParam {
    Gain,
    Pan,
    Pitch,
    Cutoff,
}

/// What the mixer gives back.
pub(crate) enum Return {
    /// A voice finished (or never started); its sound is the main thread's
    /// to drop.
    Ended { id: u64, sound: Feed },
    /// An effect removed from its bus: the main thread's to drop.
    Effect(Box<Slot>),
    /// An effect prepared for a rate the mixer no longer runs at (it
    /// changed while the effect was on its way): to prepare again and send
    /// back.
    Unprepared { bus: u16, slot: Box<Slot> },
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
    pub waited: AtomicU64,
    /// Commands the mixer is done with: applied, or found nothing to
    /// apply to. The main thread counts those it sent against it.
    pub taken: AtomicU64,
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
            waited: zero(),
            taken: zero(),
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

/// The ids of plays waiting for a slot, so a command for a voice not found
/// asks this, not every command waiting, whether its play is still to
/// come. A fixed table, open addressed: made up front, it never
/// allocates. Made to hold the schedule's whole room, which those waiting
/// never pass, it's never more than half full; were it, an id would be
/// counted rather than kept, and the waiting commands searched.
pub(crate) struct Retrying {
    /// 0: empty (no voice has id 0).
    slots: Box<[u64]>,
    /// The most kept at once: half the slots.
    held: usize,
    len: usize,
    /// Waiting plays not in the table.
    overflow: usize,
}

impl Retrying {
    /// Room for `held` at once (made a power of two, 16 or more).
    fn new(held: usize) -> Retrying {
        let held = held.max(16).next_power_of_two();
        Retrying {
            slots: vec![0; held * 2].into(),
            held,
            len: 0,
            overflow: 0,
        }
    }

    fn home(&self, id: u64) -> usize {
        // Fibonacci hashing: ids are consecutive, their slots spread.
        (id.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize & (self.slots.len() - 1)
    }

    fn contains(&self, id: u64) -> bool {
        let mask = self.slots.len() - 1;
        let mut at = self.home(id);
        loop {
            match self.slots[at] {
                0 => return false,
                held if held == id => return true,
                _ => at = (at + 1) & mask,
            }
        }
    }

    /// Keeps `id`, or counts it if the table is half full.
    fn add(&mut self, id: u64) {
        if self.len >= self.held {
            self.overflow += 1;
            return;
        }
        let mask = self.slots.len() - 1;
        let mut at = self.home(id);
        while self.slots[at] != 0 {
            at = (at + 1) & mask;
        }
        self.slots[at] = id;
        self.len += 1;
    }

    /// Lets `id` go: out of the table, or off the count.
    fn remove(&mut self, id: u64) {
        let mask = self.slots.len() - 1;
        let mut at = self.home(id);
        loop {
            match self.slots[at] {
                0 => {
                    self.overflow = self.overflow.saturating_sub(1);
                    return;
                }
                held if held == id => break,
                _ => at = (at + 1) & mask,
            }
        }
        // Each one after it in its run moves back if it may (backward
        // shift: no markers left behind, so a search ends at a gap).
        self.slots[at] = 0;
        self.len -= 1;
        let mut gap = at;
        let mut next = (at + 1) & mask;
        while self.slots[next] != 0 {
            let home = self.home(self.slots[next]);
            // It may fill the gap unless its home lies after the gap, up to
            // where it sits (cyclically).
            let stays = if gap <= next {
                gap < home && home <= next
            } else {
                gap < home || home <= next
            };
            if !stays {
                self.slots[gap] = self.slots[next];
                self.slots[next] = 0;
                gap = next;
            }
            next = (next + 1) & mask;
        }
    }
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

/// What a voice's mix is given besides its block: the frames a click-free
/// start takes, the count of streams run dry, and its own low-pass if it
/// filters.
struct Along<'a> {
    click: u32,
    underruns: &'a mut u64,
    filter: Option<&'a mut VoiceLowPass>,
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
    /// How its pan shares it between left and right.
    law: PanLaw,
    /// Whether its own low-pass (the mixer's `filters`, by its slot)
    /// filters: only then is it looked at.
    filtered: bool,
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
            law: PanLaw::Balance,
            filtered: false,
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
        along: Along<'_>,
    ) -> bool {
        let Along {
            click,
            underruns,
            mut filter,
        } = along;
        self.fresh = false;
        let fixed = self.pitch.settled();
        let step = self.step(self.pitch.value());
        if !fixed {
            let pitches = &mut scratch.values[..n];
            self.pitch.fill(pitches);
            let scale = self.base_step * ONE as f64;
            for (step, &pitch) in scratch.steps[..n].iter_mut().zip(pitches.iter()) {
                // Kept from zero as a float: the loop vectorises.
                *step = (scale * f64::from(pitch)).max(1.) as u64;
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
                if let Some(filter) = &mut filter {
                    filter.cutoff.advance(n);
                }
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
        if let Some(filter) = filter {
            // Its own low-pass, before its gain and pan: a mono sound's one
            // channel, a stereo sound's two.
            let (l, r) = (&mut scratch.left[..got], &mut scratch.right[..got]);
            filter.run(l, (sound.channels != 1).then_some(r), n);
        }
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
                let (pl, pr) = self.law.gains(self.pan.value());
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
                let (pl, pr) = self.law.gains(self.pan.value());
                for i in 0..got {
                    left[i] += source_left[i] * gains[i] * pl;
                    right[i] += source_right[i] * gains[i] * pr;
                }
            } else {
                let pans = &mut scratch.values[..n];
                self.pan.fill(pans);
                // A loop for each law, chosen once: either vectorises, a
                // choice inside it wouldn't.
                match self.law {
                    PanLaw::Balance => {
                        for i in 0..got {
                            let (pl, pr) = pan_gains(pans[i]);
                            left[i] += source_left[i] * gains[i] * pl;
                            right[i] += source_right[i] * gains[i] * pr;
                        }
                    }
                    PanLaw::EqualPower => equal_power_into(
                        &source_left[..got],
                        &source_right[..got],
                        &gains[..got],
                        &pans[..got],
                        &mut left[..got],
                        &mut right[..got],
                    ),
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

/// How a voice's pan shares it between left and right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanLaw {
    /// A balance: the near side stays whole and the far one falls along an
    /// equal-power curve, so a sound in the middle plays as it was made.
    /// For sounds placed on a stage: UI, music.
    #[default]
    Balance,
    /// Equal power: a quarter turn of cosine and sine, the same power
    /// wherever it is, -3 dB a side in the middle. For sounds in space,
    /// moving round the listener.
    EqualPower,
}

impl PanLaw {
    #[inline]
    fn gains(self, pan: f32) -> (f32, f32) {
        match self {
            PanLaw::Balance => pan_gains(pan),
            PanLaw::EqualPower => equal_power_once(pan),
        }
    }
}

#[inline]
fn equal_power(pan: f32) -> (f32, f32) {
    let [l, r] = crate::spatial::equal_power(pan);
    (l, r)
}

#[inline(never)]
fn equal_power_once(pan: f32) -> (f32, f32) {
    equal_power(pan)
}

/// A voice's frames into its bus by the equal-power law, its pan moving:
/// out of line, so a mix that doesn't use it carries none of its code.
#[inline(never)]
fn equal_power_into(
    source_left: &[f32],
    source_right: &[f32],
    gains: &[f32],
    pans: &[f32],
    left: &mut [f32],
    right: &mut [f32],
) {
    for i in 0..left.len().min(right.len()) {
        let (pl, pr) = equal_power(pans[i]);
        left[i] += source_left[i] * gains[i] * pl;
        right[i] += source_right[i] * gains[i] * pr;
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

/// A bus's effects, and what deciding when they run keeps: apart from
/// [`Bus`], which the voices' loop reads, so a mix without effects is the
/// same code as before there were any.
struct Chain {
    /// In order; room for [`MAX_EFFECTS`] made up front. Boxed: each
    /// arrives in a box and leaves in it, which unboxing here would free
    /// on the audio thread.
    #[allow(clippy::vec_box)]
    effects: Vec<Box<Slot>>,
    /// Their running on silence, decided every [`CELL`] frames of the
    /// clock: the frame until which they may still sound with no input
    /// (where the last cell fed ended, plus the longest tail); whether
    /// they run in this cell; whether this cell was fed.
    horizon: u64,
    active: bool,
    fed: bool,
    /// Effects anywhere that take this bus's level as their side chain.
    keyed: u32,
}

impl Chain {
    fn new() -> Chain {
        Chain {
            effects: Vec::with_capacity(MAX_EFFECTS),
            horizon: 0,
            active: false,
            fed: false,
            keyed: 0,
        }
    }
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
    /// Plays that found no slot, and commands sent for their voices, in
    /// the order sent: only the first is tried, every [`RETRY`] frames,
    /// and those behind it wait their turn, so a crowd waiting costs a
    /// block one try, not one each. Made with room for the schedule's
    /// whole room: what's here left the schedule, and together they never
    /// pass it.
    waiting: VecDeque<Command>,
    /// When the first waiting is tried again.
    retry_at: u64,
    retrying: Box<Retrying>,
    /// Commands the schedule holds at most: its room, as configured (the
    /// heap's own capacity may be more).
    room: usize,
    seq: u64,
    voices: Box<[Voice]>,
    /// Each voice slot's own low-pass, by the same index: apart from the
    /// voices, so a voice without one is as before.
    filters: Box<[VoiceLowPass]>,
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
    /// Work the tests count, not time: plays tried, and commands looked
    /// through for a waiting play.
    #[cfg(test)]
    pub(crate) work: [std::cell::Cell<u64>; 2],
    /// Times a play found no free slot and waited.
    waited: u64,
    /// Commands done with.
    taken: u64,
    /// Frames of the shortest de-click fade (1 ms), and of a steal's or a
    /// mute's (5 ms).
    click: u32,
    declick: u32,
    /// Each bus's level (after its gain) over the block being mixed, and
    /// over the one before: what side chains read, so no bus waits on
    /// another.
    levels: [f32; MAX_BUSES],
    heard: [f32; MAX_BUSES],
    /// Effects with a side chain, anywhere.
    keyed: u32,
    /// Each bus's effects, by its index.
    chains: Box<[Chain]>,
    /// Effects on every bus.
    effects: usize,
    /// A bus's block interleaved, for its effects.
    frames: Box<[f32; 2 * MAX_BLOCK]>,
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
            waiting: VecDeque::with_capacity(setup.commands),
            retry_at: 0,
            retrying: Box::new(Retrying::new(setup.commands)),
            room: setup.commands,
            seq: 0,
            voices: (0..setup.voices + FADING_SLOTS)
                .map(|_| Voice::free())
                .collect(),
            filters: (0..setup.voices + FADING_SLOTS)
                .map(|_| VoiceLowPass::new(setup.rate))
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
            frames: Box::new([0.; 2 * MAX_BLOCK]),
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
            #[cfg(test)]
            work: Default::default(),
            waited: 0,
            taken: 0,
            click: (setup.rate / 1000).max(1),
            declick: (setup.rate / 200).max(1),
            levels: [0.; MAX_BUSES],
            heard: [0.; MAX_BUSES],
            keyed: 0,
            chains: (0..MAX_BUSES).map(|_| Chain::new()).collect(),
            effects: 0,
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
        self.voices.iter().any(|v| v.id != 0)
            || !self.schedule.is_empty()
            || !self.waiting.is_empty()
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
        for slot in self.chains.iter_mut().flat_map(|c| c.effects.iter_mut()) {
            slot.prepare(rate);
        }
        for filter in self.filters.iter_mut() {
            filter.set_rate(rate);
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
            let retry = (!self.waiting.is_empty()).then_some(self.retry_at);
            if let Some(next) = self
                .schedule
                .peek()
                .map(|s| s.time)
                .into_iter()
                .chain(retry)
                .min()
            {
                // Kept to a block in u64 before it's a usize: a command
                // 2^32 frames away is no block of 0 on a 32-bit target.
                let until = next.saturating_sub(self.now).clamp(1, MAX_BLOCK as u64);
                n = n.min(until as usize);
            }
            if self.effects > 0 {
                // Effects decide on the clock's cells: no block spans two.
                n = n.min((CELL - self.now % CELL) as usize);
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
        let ringing = self.effects > 0
            && self.chains[..self.bus_count]
                .iter()
                .any(|c| c.horizon > self.now);
        if self.voices.iter().any(|v| v.id != 0)
            || !quiet
            || ringing
            || !self.schedule.is_empty()
            || !self.waiting.is_empty()
        {
            meter.audible.store(self.now, Ordering::Relaxed);
        }
        if let Some(timing) = timing {
            // A device's clock is its own: kept from wrapping at its end.
            let after = |frames: u64| {
                timing
                    .heard
                    .saturating_add(frames.saturating_mul(1_000_000_000) / u64::from(self.rate))
            };
            meter.block_asked.store(timing.asked, Ordering::Relaxed);
            meter.block_heard.store(timing.heard, Ordering::Relaxed);
            if let Some(started) = self.started {
                let latency = self.limiter.as_ref().map_or(0, Limiter::latency) as u64;
                meter.play_asked.store(timing.asked, Ordering::Relaxed);
                meter.play_heard.store(
                    after(started.saturating_sub(first) + latency),
                    Ordering::Release,
                );
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
        meter.waited.store(self.waited, Ordering::Relaxed);
        meter.taken.store(self.taken, Ordering::Release);
        meter.now.store(self.now, Ordering::Release);
    }

    /// Moves commands from the queue into the schedule, as many as fit.
    fn collect(&mut self) {
        while self.schedule.len() + self.waiting.len() < self.room {
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
        // Those waiting were sent before anything due now: their turn first.
        if !self.waiting.is_empty() && self.retry_at <= self.now {
            self.retry_waiting();
        }
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
            // Behind those waiting: a play, while any wait, and anything
            // for a voice whose play waits.
            let behind = match command {
                Command::Play { .. } => !self.waiting.is_empty(),
                Command::Stop { id, .. } | Command::Voice { id, .. } => self.waiting(id),
                _ => false,
            };
            if behind {
                self.wait(command);
                continue;
            }
            match self.apply(command) {
                Ok(()) => self.taken += 1,
                Err(command @ Command::Play { .. }) => self.wait(command),
                Err(command) => {
                    // Nowhere yet to give back what it took off: tried again
                    // a little later. The heap has room: this one just left.
                    self.schedule.push(Scheduled {
                        time: self.now.saturating_add(RETRY),
                        seq,
                        command,
                    });
                }
            }
        }
    }

    /// Puts `command` at the back of those waiting.
    fn wait(&mut self, command: Command) {
        if self.waiting.is_empty() {
            self.retry_at = self.now.saturating_add(RETRY);
        }
        if let Command::Play { id, .. } = command {
            self.waited += 1;
            self.retrying.add(id);
        }
        // Room made for the schedule's whole room: no allocation.
        self.waiting.push_back(command);
    }

    /// Tries those waiting in order, until one still can't be done.
    fn retry_waiting(&mut self) {
        while let Some(command) = self.waiting.pop_front() {
            let play = match command {
                Command::Play { id, .. } => Some(id),
                _ => None,
            };
            match self.apply(command) {
                Ok(()) => {
                    self.taken += 1;
                    if let Some(id) = play {
                        self.retrying.remove(id);
                    }
                }
                Err(command) => {
                    // Still no slot: the rest wait behind it.
                    self.waiting.push_front(command);
                    self.retry_at = self.now.saturating_add(RETRY);
                    return;
                }
            }
        }
    }

    fn voice(&mut self, id: u64) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id && !v.ended)
    }

    /// Whether the play of voice `id` waits for a slot: then what's sent
    /// for it after waits too, rather than be lost.
    fn waiting(&self, id: u64) -> bool {
        self.retrying.contains(id)
            || (self.retrying.overflow > 0
                && self.waiting.iter().any(|c| {
                    #[cfg(test)]
                    self.work[1].set(self.work[1].get() + 1);
                    matches!(c, Command::Play { id: play, .. } if *play == id)
                }))
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
                } else if self.waiting(id) {
                    return Err(command);
                }
            }
            Command::Voice {
                id,
                param,
                value,
                ramp,
            } => {
                if self.voice(id).is_none() && self.waiting(id) {
                    return Err(command);
                }
                let (frames, curve) = (ramp.frames(self.rate), ramp.curve);
                let slot = self.voices.iter().position(|v| v.id == id && !v.ended);
                if let Some(slot) = slot {
                    let voice = &mut self.voices[slot];
                    match param {
                        VoiceParam::Cutoff => {
                            self.filters[slot].cutoff.set(value, frames, curve);
                            voice.filtered = self.filters[slot].active();
                        }
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
            Command::AddEffect { bus, slot } => return self.add_effect(bus, slot),
            Command::RemoveEffect { id } => {
                let found = self.chains.iter().enumerate().find_map(|(b, chain)| {
                    let at = chain.effects.iter().position(|slot| slot.id == id)?;
                    Some((b, at))
                });
                if let Some((b, at)) = found {
                    let slot = self.chains[b].effects.remove(at);
                    let side = slot.side;
                    match self.returns.push(Return::Effect(slot)) {
                        Ok(()) => {
                            self.effects -= 1;
                            if let Some(side) = side {
                                self.chains[usize::from(side)].keyed -= 1;
                                self.keyed -= 1;
                            }
                        }
                        Err(Return::Effect(slot)) => {
                            // No room to give it back: kept, and tried again.
                            self.chains[b].effects.insert(at, slot);
                            return Err(Command::RemoveEffect { id });
                        }
                        Err(_) => unreachable!("an effect is given back as one"),
                    }
                }
            }
            Command::EffectParam {
                id,
                index,
                value,
                ramp,
            } => {
                let (frames, curve) = (ramp.frames(self.rate), ramp.curve);
                let param = self
                    .chains
                    .iter_mut()
                    .flat_map(|chain| chain.effects.iter_mut())
                    .find(|slot| slot.id == id)
                    .and_then(|slot| slot.params.get_mut(usize::from(index)));
                if let Some(param) = param {
                    let value = if value.is_nan() { param.min } else { value };
                    // The curves for gains take values as gains: none
                    // below zero. A range that goes there moves straight.
                    let curve = if param.min < 0. { Curve::Linear } else { curve };
                    param
                        .smoothed
                        .set(value.clamp(param.min, param.max), frames, curve);
                }
            }
        }
        Ok(())
    }

    /// Puts `slot` at the end of `bus`'s chain, or gives it back to be
    /// prepared again if the rate changed on its way.
    fn add_effect(&mut self, bus: u16, slot: Box<Slot>) -> Result<(), Command> {
        let unprepared = slot.rate != self.rate;
        // The main thread keeps count: a full chain never gets another.
        let full = self.chains[usize::from(bus)].effects.len() >= MAX_EFFECTS;
        if unprepared || full {
            let back = if unprepared {
                Return::Unprepared { bus, slot }
            } else {
                Return::Effect(slot)
            };
            return self.returns.push(back).map_err(|back| match back {
                Return::Unprepared { slot, .. } | Return::Effect(slot) => {
                    Command::AddEffect { bus, slot }
                }
                Return::Ended { .. } => unreachable!("an effect is given back as one"),
            });
        }
        if let Some(side) = slot.side {
            self.chains[usize::from(side)].keyed += 1;
            self.keyed += 1;
        }
        self.chains[usize::from(bus)].effects.push(slot);
        self.effects += 1;
        Ok(())
    }

    fn play(&mut self, command: Command) -> Result<(), Command> {
        #[cfg(test)]
        self.work[0].set(self.work[0].get() + 1);
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
            law,
            cutoff,
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
                        law,
                        cutoff,
                    }),
                    Err(_) => unreachable!("a dropped play is given back as one"),
                };
            }
            victim.envelope.set(0., self.declick, Curve::Linear);
            victim.stopping = true;
            self.stolen += 1;
        }
        let (rate, click, interpolation) = (self.rate, self.click, self.interpolation);
        let Some(slot) = self.voices.iter().position(|v| v.id == 0) else {
            return Err(command);
        };
        self.filters[slot].reset(cutoff);
        let filtered = self.filters[slot].active();
        let voice = &mut self.voices[slot];
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
            law,
            filtered,
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
        for (slot, voice) in self.voices.iter_mut().enumerate() {
            if voice.id == 0 {
                continue;
            }
            if !voice.ended {
                let bus = &mut self.buses[usize::from(voice.bus)];
                bus.touch(n);
                let filter = if voice.filtered {
                    let filter = &mut self.filters[slot];
                    // Open and still: out of the way from now on.
                    voice.filtered = filter.active();
                    voice.filtered.then_some(filter)
                } else {
                    None
                };
                if !voice.mix(
                    n,
                    &mut self.scratch,
                    &mut bus.left[..n],
                    &mut bus.right[..n],
                    Along {
                        click: self.click,
                        underruns: &mut self.underruns,
                        filter,
                    },
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
                    Err(_) => unreachable!("an ended voice is given back as one"),
                }
            }
        }
        // With no effect anywhere, the buses' loop is the plain one: what
        // effects need costs a mix without them nothing.
        if self.effects > 0 {
            self.sum_with_effects(n);
        } else {
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

    /// The buses' loop with effects somewhere: each bus through its own
    /// before it sums into its parent, its level kept where a side chain
    /// reads it, then master through its own.
    #[inline(never)]
    fn sum_with_effects(&mut self, n: usize) {
        if self.keyed > 0 {
            self.heard = self.levels;
        }
        for b in (1..self.bus_count).rev() {
            let (below, above) = self.buses.split_at_mut(b);
            let (bus, chain) = (&mut above[0], &mut self.chains[b]);
            if !bus.used {
                continue;
            }
            if !chain.effects.is_empty() {
                run_effects(bus, chain, n, self.now, &mut self.frames, &self.heard);
            }
            if !bus.touched {
                bus.gain.advance(n);
                bus.gate.advance(n);
                self.levels[b] = 0.;
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
            if chain.keyed > 0 {
                self.levels[b] = level(bus, n);
            }
        }
        let (master, chain) = (&mut self.buses[0], &mut self.chains[0]);
        if !chain.effects.is_empty() {
            run_effects(master, chain, n, self.now, &mut self.frames, &self.heard);
        }
        if chain.keyed > 0 {
            self.levels[0] = if master.touched { level(master, n) } else { 0. };
        }
    }
}

/// Runs `bus`'s effects on its block, which lies within one of the
/// clock's cells. At a cell's first frame the bus decides whether they run
/// through it: yes if the cell before was fed, or a tail (read there) may
/// still sound. A cell they don't run in starts them on the first frame
/// that brings sound. Decided only on the clock, by what's heard, which
/// frames an effect runs on doesn't depend on block sizes; otherwise their
/// parameters move on and nothing runs: silence in, silence out. Not
/// inlined: a mix with no effects doesn't carry it.
#[inline(never)]
fn run_effects(
    bus: &mut Bus,
    chain: &mut Chain,
    n: usize,
    now: u64,
    frames: &mut [f32; 2 * MAX_BLOCK],
    heard: &[f32; MAX_BUSES],
) {
    if now.is_multiple_of(CELL) {
        if chain.fed {
            let tail = chain.effects.iter().map(|slot| slot.tail()).max();
            // A tail as long as time (`Duration::MAX`) rings for ever.
            chain.horizon = now.saturating_add(tail.unwrap_or(0));
        }
        chain.active = now < chain.horizon;
        chain.fed = false;
    }
    // Sound in this block: its first frame, if any.
    let first = if bus.touched {
        let quiet = |(l, r): (&f32, &f32)| *l == 0. && *r == 0.;
        bus.left[..n]
            .iter()
            .zip(&bus.right[..n])
            .position(|frame| !quiet(frame))
    } else {
        None
    };
    chain.fed |= first.is_some();
    let from = if chain.active {
        0
    } else if let Some(first) = first {
        chain.active = true;
        first
    } else {
        for slot in chain.effects.iter_mut() {
            slot.skip(n);
        }
        return;
    };
    bus.touch(n);
    let pairs = &mut frames.as_chunks_mut::<2>().0[..n];
    for (pair, (l, r)) in pairs.iter_mut().zip(bus.left.iter().zip(bus.right.iter())) {
        *pair = [*l, *r];
    }
    for slot in chain.effects.iter_mut() {
        let side = slot.side.map(|s| heard[usize::from(s)]);
        slot.skip(from);
        slot.process(&mut frames[from * 2..], n - from, now + from as u64, side);
    }
    // Back out finite and bounded, whatever an effect made: every source of
    // sound (a voice, or a bus's effects) is then within its bound, and so
    // is any sum of them through the tree. Checked as bits without their
    // sign, an integer maximum, which vectorises (not a number and the
    // infinities are above any finite value), and mended only in a block
    // that needs it.
    let block = &mut frames[..n * 2];
    let loudest = block
        .iter()
        .fold(0u32, |m, s| m.max(s.to_bits() & 0x7fff_ffff));
    if loudest > EFFECT_LIMIT.to_bits() {
        for s in block.iter_mut() {
            *s = if s.is_nan() {
                0.
            } else {
                s.clamp(-EFFECT_LIMIT, EFFECT_LIMIT)
            };
        }
    }
    let pairs = &frames.as_chunks::<2>().0[..n];
    for (pair, (l, r)) in pairs
        .iter()
        .zip(bus.left.iter_mut().zip(bus.right.iter_mut()))
    {
        [*l, *r] = *pair;
    }
}

/// The root mean square of `bus`'s block, both sides, after its gain as
/// it ends the block.
#[inline(never)]
fn level(bus: &Bus, n: usize) -> f32 {
    let sum: f32 = bus.left[..n]
        .iter()
        .zip(&bus.right[..n])
        .map(|(l, r)| l * l + r * r)
        .sum();
    (sum / (2 * n) as f32).sqrt() * bus.gain.value() * bus.gate.value()
}

/// The bus's gain times its gate for each of the next `n` frames.
#[inline(always)]
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
#[inline(always)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_retried_plays_table_holds_what_a_set_would() {
        let mut table = Retrying::new(2_048);
        let mut set = HashSet::new();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = 1u64;
        for _ in 0..200_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if !seed.is_multiple_of(3) && set.len() < table.held {
                table.add(next);
                set.insert(next);
                next += 1;
            } else if let Some(&id) = set.iter().next() {
                table.remove(id);
                set.remove(&id);
            }
            let probe = seed % next.max(1);
            assert_eq!(table.contains(probe), set.contains(&probe), "id {probe}");
        }
        assert_eq!(table.len, set.len());
        assert_eq!(table.overflow, 0);
        // Past what it holds: counted, and uncounted as they go.
        for id in &set {
            table.remove(*id);
        }
        for id in 0..table.held as u64 + 5 {
            table.add(next + id);
        }
        assert_eq!((table.len, table.overflow), (table.held, 5));
        table.remove(u64::MAX);
        assert_eq!(table.overflow, 4);
    }
}
