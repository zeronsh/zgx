//! The mixer: what runs on the audio thread. It applies each command at
//! its frame, mixes voices into their buses, buses into their parents, and
//! master through the limiter. It touches only memory made before it ran
//! and never locks, allocates, frees or calls the system: sounds it's done
//! with go back to the main thread to be dropped there.
use crate::limiter::Limiter;
use crate::queue::{Consumer, Producer};
use crate::ramp::{Curve, Smoothed};
use crate::resample::{Interpolation, ONE, Position, Region, Resampler, Step};
use crate::sound::SoundData;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The most frames mixed at once; a device's longer blocks are mixed in
/// pieces of this.
pub(crate) const MAX_BLOCK: usize = 128;
/// Buses at most, master included.
pub(crate) const MAX_BUSES: usize = 32;
/// Slots past the voice limit, for voices fading out after being stolen.
pub(crate) const FADING_SLOTS: usize = 16;
/// Frames before a play that found no slot tries again.
const RETRY: u64 = 64;

/// What the main thread asks of the mixer.
pub(crate) enum Command {
    Play {
        id: u64,
        sound: Arc<SoundData>,
        bus: u16,
        gain: f32,
        pan: f32,
        pitch: f32,
        region: Region,
        priority: i8,
        /// Frames of fade in and their curve; `None`, a click-free start.
        fade_in: Option<(u32, Curve)>,
    },
    Stop {
        id: u64,
        frames: u32,
        curve: Curve,
    },
    Voice {
        id: u64,
        param: VoiceParam,
        value: f32,
        frames: u32,
        curve: Curve,
    },
    BusGain {
        bus: u16,
        value: f32,
        frames: u32,
        curve: Curve,
    },
    Mute {
        bus: u16,
        on: bool,
    },
    Solo {
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
    Ended { id: u64, sound: Arc<SoundData> },
}

/// Counters the mixer publishes after each block and anyone reads.
#[derive(Default)]
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
    sound: Option<Arc<SoundData>>,
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
    ) -> bool {
        self.fresh = false;
        let sound = self
            .sound
            .as_deref()
            .expect("a playing voice has its sound");
        let got = if self.pitch.settled() {
            let step = Step::Fixed(self.step(self.pitch.value()));
            self.resampler.render(
                sound,
                self.region,
                &mut self.at,
                step,
                &mut scratch.left[..n],
                &mut scratch.right[..n],
            )
        } else {
            let pitches = &mut scratch.values[..n];
            self.pitch.fill(pitches);
            let scale = self.base_step * ONE as f64;
            for (step, &pitch) in scratch.steps[..n].iter_mut().zip(pitches.iter()) {
                *step = ((scale * f64::from(pitch)) as u64).max(1);
            }
            let step = Step::PerFrame(&scratch.steps[..n]);
            self.resampler.render(
                sound,
                self.region,
                &mut self.at,
                step,
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
        if self.gain.settled() && self.envelope.settled() && self.pan.settled() {
            let gain = self.gain.value() * self.envelope.value();
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
            if !self.envelope.settled() {
                let envelope = &mut scratch.values[..n];
                self.envelope.fill(envelope);
                for (g, e) in gains.iter_mut().zip(envelope.iter()) {
                    *g *= e;
                }
            } else {
                let envelope = self.envelope.value();
                if envelope != 1. {
                    gains.iter_mut().for_each(|g| *g *= envelope);
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
        let faded = self.stopping && self.envelope.settled() && self.envelope.value() == 0.;
        got == n && !faded
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
    /// Commands the schedule holds at most: its room, as configured (the
    /// heap's own capacity may be more).
    room: usize,
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
    /// Times a play found no free slot and waited.
    waited: u64,
    /// Commands done with.
    taken: u64,
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
            room: setup.commands,
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
            waited: 0,
            taken: 0,
            click: (setup.rate / 1000).max(1),
            declick: (setup.rate / 200).max(1),
        }
    }

    /// Frames the output lags a command's time: the limiter's look-ahead.
    pub fn latency(&self) -> usize {
        self.limiter.as_ref().map_or(0, Limiter::latency)
    }

    /// Mixes the next `out.len() / channels` frames into `out`, interleaved.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let mut done = 0;
        self.collect();
        while done < frames {
            self.apply_due();
            let mut n = (frames - done).min(MAX_BLOCK);
            if let Some(next) = self.schedule.peek() {
                // Kept to a block in u64 before it's a usize: a command
                // 2^32 frames away is no block of 0 on a 32-bit target.
                let until = next
                    .time
                    .saturating_sub(self.now)
                    .clamp(1, MAX_BLOCK as u64);
                n = n.min(until as usize);
            }
            self.mix(n);
            let master = &self.buses[0];
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
        while self.schedule.len() < self.room {
            let Some((time, command)) = self.commands.pop() else {
                break;
            };
            self.seq += 1;
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
            let result = self.apply(command);
            if result.is_ok() {
                self.taken += 1;
            }
            if let Err(command) = result {
                // No slot yet (or, for a stop or a change, its play has
                // none yet): tried again a little later, in the order sent.
                // The heap has room: this one just left it.
                if matches!(command, Command::Play { .. }) {
                    self.waited += 1;
                }
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

    /// Whether the play of voice `id` waits in the schedule for a slot:
    /// then what's sent for it after waits too, rather than be lost.
    fn waiting(&self, id: u64) -> bool {
        self.schedule
            .iter()
            .any(|s| matches!(s.command, Command::Play { id: play, .. } if play == id))
    }

    fn apply(&mut self, command: Command) -> Result<(), Command> {
        match command {
            Command::Play { .. } => return self.play(command),
            Command::Stop { id, frames, curve } => {
                let click = self.click;
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
                frames,
                curve,
            } => {
                if self.voice(id).is_none() && self.waiting(id) {
                    return Err(command);
                }
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
            Command::BusGain {
                bus,
                value,
                frames,
                curve,
            } => {
                self.buses[usize::from(bus)]
                    .gain
                    .set(value.max(0.), frames, curve);
            }
            Command::Mute { bus, on } => {
                self.buses[usize::from(bus)].muted = on;
                self.gate();
            }
            Command::Solo { bus, on } => {
                self.buses[usize::from(bus)].soloed = on;
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
        let start = sound.samples[..sound.channels.min(sound.samples.len())]
            .iter()
            .fold(0f32, |p, s| p.max(s.abs()));
        *voice = Voice {
            id,
            sound: None,
            bus,
            at: 0,
            base_step: f64::from(sound.rate) / f64::from(rate),
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
            Some((frames, curve)) => {
                voice.envelope = Smoothed::new(0.);
                voice.envelope.set(1., frames, curve);
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
            let target = if bus.muted || !audible { 0. } else { 1. };
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
                    Ok(()) => voice.id = 0,
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
