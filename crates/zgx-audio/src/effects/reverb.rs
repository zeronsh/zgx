//! A reverb: a feedback delay network of eight lines (Jot's), their sum
//! fed back through a Householder matrix, each line's loop damped by a
//! one-pole low-pass and given the gain that makes its sound decay at the
//! time asked for.
use crate::{Context, Effect, Format, Parameter};
use std::time::Duration;

/// Lines in the network.
const LINES: usize = 8;

/// Each line's length at size 1, in milliseconds: spread over 30 to 74 ms,
/// and in frames at 48 kHz prime, so their echoes rarely line up.
const LENGTHS_MS: [f32; LINES] = [
    29.729_166, 37.020_832, 41.520_832, 44.854_168, 52.895_832, 59.229_168, 66.854_164, 73.479_164,
];

/// Frames of the clock between remakings of the gains and lengths while
/// they move: made only there, a block split anywhere gives the same sound.
const STEP: u64 = 32;

/// A room's reverberation, from a feedback delay network: eight lines whose
/// sum is fed back through a Householder matrix, each loop damped.
///
/// Parameters: [`SIZE`](Self::SIZE), the lines' lengths over their
/// default (0.25 to 2: a cupboard to a hall); [`DECAY`](Self::DECAY), the
/// seconds its sound takes to fall 60 dB (0.1 to 20); [`DAMPING`](Self::DAMPING),
/// how much faster high frequencies die (0 to 1); [`PRE_DELAY`](Self::PRE_DELAY),
/// seconds before it begins (0 to 0.25); [`WET`](Self::WET) and [`DRY`](Self::DRY),
/// its gain and the sound's own (0 to 1).
#[derive(Clone, Debug)]
pub struct Reverb {
    params: [Parameter; 6],
    rate: f32,
    /// The eight lines, each a ring of `ring` frames, written together at
    /// `at`.
    lines: Vec<[f32; LINES]>,
    ring: usize,
    at: usize,
    /// Each loop's damping state.
    damped: [f32; LINES],
    /// The pre-delay: stereo frames, written at `pre_at`.
    pre: Vec<[f32; 2]>,
    pre_at: usize,
    /// As last made: each line's length in frames (whole, or not while the
    /// size moves), its loop gain, the damping coefficient, and the
    /// pre-delay in frames.
    lengths: [f32; LINES],
    gains: [f32; LINES],
    damping: f32,
    pre_frames: usize,
    /// The size and decay those were made for (NaN: never).
    made: [f32; 3],
}

impl Reverb {
    pub const SIZE: usize = 0;
    pub const DECAY: usize = 1;
    pub const DAMPING: usize = 2;
    pub const PRE_DELAY: usize = 3;
    pub const WET: usize = 4;
    pub const DRY: usize = 5;

    /// A room of `size` (1: a large room) whose sound falls 60 dB in `decay`
    /// seconds.
    pub fn new(size: f32, decay: f32) -> Reverb {
        Reverb {
            params: [
                Parameter::new("size", size.clamp(0.25, 2.), 0.25, 2.),
                Parameter::new("decay", decay.clamp(0.1, 20.), 0.1, 20.),
                Parameter::new("damping", 0.5, 0., 1.),
                Parameter::new("pre-delay", 0.01, 0., 0.25),
                Parameter::new("wet", 0.3, 0., 1.),
                Parameter::new("dry", 1., 0., 1.),
            ],
            rate: 48_000.,
            lines: Vec::new(),
            ring: 0,
            at: 0,
            damped: [0.; LINES],
            pre: Vec::new(),
            pre_at: 0,
            lengths: [0.; LINES],
            gains: [0.; LINES],
            damping: 0.,
            pre_frames: 0,
            made: [f32::NAN; 3],
        }
    }

    /// How much faster high frequencies die, from 0 to 1.
    pub fn damping(mut self, damping: f32) -> Reverb {
        self.params[Self::DAMPING].default = damping.clamp(0., 1.);
        self
    }

    /// Seconds before it begins.
    pub fn pre_delay(mut self, seconds: f32) -> Reverb {
        self.params[Self::PRE_DELAY].default = seconds.clamp(0., 0.25);
        self
    }

    /// Its gain, and the sound's own.
    pub fn mix(mut self, wet: f32, dry: f32) -> Reverb {
        self.params[Self::WET].default = wet.clamp(0., 1.);
        self.params[Self::DRY].default = dry.clamp(0., 1.);
        self
    }

    fn pre_frames_for(&mut self, seconds: f32) -> usize {
        self.pre_frames = ((seconds * self.rate) as usize).min(self.pre.len() - 1);
        self.pre_frames
    }

    /// One frame through the network, its lines' outputs `out` and its
    /// input `input` (pre-delayed): each loop damped and decayed, all
    /// mixed by the Householder matrix (each less twice their mean), the
    /// input added and written back. Returns left (from the even lines)
    /// and right (from the odd), with signs that keep the two apart.
    #[inline(always)]
    fn step(&mut self, out: [f32; LINES], input: [f32; 2]) -> [f32; 2] {
        let mut fed = [0f32; LINES];
        for l in 0..LINES {
            self.damped[l] += self.damping * (out[l] - self.damped[l]);
            fed[l] = self.damped[l] * self.gains[l];
        }
        let mean = fed.iter().sum::<f32>() * (2. / LINES as f32);
        let into = [input[0] * 0.5, input[1] * 0.5];
        self.lines[self.at] = std::array::from_fn(|l| fed[l] - mean + into[l % 2]);
        self.at += 1;
        if self.at == self.ring {
            self.at = 0;
        }
        [
            (out[0] - out[2] + out[4] - out[6]) * 0.5,
            (out[1] - out[3] + out[5] - out[7]) * 0.5,
        ]
    }

    /// The block with nothing moving: every line a whole number of frames
    /// long, read at a cursor of its own. In runs that stop short of any
    /// cursor going round its ring, so within one each is where it began
    /// plus the frame: no division, no wrapping a frame, the damping kept
    /// in registers.
    fn steady(&mut self, frames: &mut [[f32; 2]], pre_frames: usize, wet: f32, dry: f32) {
        let (ring, len) = (self.ring, self.pre.len());
        let lengths = self.lengths.map(|l| l as usize);
        let mut reads = lengths.map(|l| (self.at + ring - l) % ring);
        let mut late = (self.pre_at + len - pre_frames) % len;
        let (gains, damping) = (self.gains, self.damping);
        let mut damped = self.damped;
        let mut done = 0;
        while done < frames.len() {
            let n = reads
                .iter()
                .fold((frames.len() - done).min(ring - self.at), |n, &r| {
                    n.min(ring - r)
                })
                .min(len - self.pre_at)
                .min(len - late);
            for (k, frame) in frames[done..done + n].iter_mut().enumerate() {
                self.pre[self.pre_at + k] = *frame;
                let input = self.pre[late + k];
                let out: [f32; LINES] = std::array::from_fn(|l| self.lines[reads[l] + k][l]);
                let mut fed = [0f32; LINES];
                for l in 0..LINES {
                    damped[l] += damping * (out[l] - damped[l]);
                    fed[l] = damped[l] * gains[l];
                }
                let mean = fed.iter().sum::<f32>() * (2. / LINES as f32);
                let into = [input[0] * 0.5, input[1] * 0.5];
                self.lines[self.at + k] = std::array::from_fn(|l| fed[l] - mean + into[l % 2]);
                let left = (out[0] - out[2] + out[4] - out[6]) * 0.5;
                let right = (out[1] - out[3] + out[5] - out[7]) * 0.5;
                *frame = [dry * frame[0] + wet * left, dry * frame[1] + wet * right];
            }
            let on = |at: usize, size: usize| if at + n == size { 0 } else { at + n };
            self.at = on(self.at, ring);
            self.pre_at = on(self.pre_at, len);
            late = on(late, len);
            reads = reads.map(|r| on(r, ring));
            done += n;
        }
        self.damped = damped;
    }

    /// Lengths, gains and damping for these parameters. A length is whole
    /// unless `fractional` (while the size moves, so it glides).
    fn make(&mut self, size: f32, decay: f32, damping: f32, fractional: bool) {
        if [size, decay, damping] == self.made && !fractional {
            return;
        }
        self.made = [size, decay, damping];
        let top = (self.ring - 2) as f32;
        for ((ms, length), gain) in LENGTHS_MS
            .iter()
            .zip(self.lengths.iter_mut())
            .zip(self.gains.iter_mut())
        {
            let frames = (ms * 0.001 * size * self.rate).clamp(1., top);
            let frames = if fractional { frames } else { frames.round() };
            *length = frames;
            // -60 dB over `decay` seconds: -3 decades a decay, this line's
            // share of it each time round.
            *gain = 10f32.powf(-3. * frames / (self.rate * decay));
        }
        // The loop's low-pass: 1 passes all; less, the highs die first.
        self.damping = 1. - 0.9 * damping;
    }
}

impl Effect for Reverb {
    fn parameters(&self) -> &[Parameter] {
        &self.params
    }

    fn prepare(&mut self, format: Format, _max_block: usize) {
        self.rate = format.rate as f32;
        // The longest line at the largest size, and a frame to read past.
        let longest = LENGTHS_MS[LINES - 1] * 0.001 * 2. * self.rate;
        self.ring = longest.ceil() as usize + 2;
        self.lines = vec![[0.; LINES]; self.ring];
        self.at = 0;
        self.damped = [0.; LINES];
        self.pre = vec![[0.; 2]; (0.25 * self.rate).ceil() as usize + 1];
        self.pre_at = 0;
        self.made = [f32::NAN; 3];
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let frames = block.as_chunks_mut::<2>().0;
        let moving = |p: usize| context.values(p).is_some();
        let fractional = moving(Self::SIZE);
        let reshaping = fractional || moving(Self::DECAY) || moving(Self::DAMPING);
        if !reshaping {
            let [size, decay, damping] = [0, 1, 2].map(|p| context.value(p));
            self.make(size, decay, damping, false);
            if !(3..6).any(moving) {
                let pre_frames = self.pre_frames_for(context.value(Self::PRE_DELAY));
                let [wet, dry] = [Self::WET, Self::DRY].map(|p| context.value(p));
                self.steady(frames, pre_frames, wet, dry);
                return;
            }
        }
        let start = context.time().0;
        for (i, frame) in frames.iter_mut().enumerate() {
            if reshaping && ((start + i as u64).is_multiple_of(STEP) || self.made[0].is_nan()) {
                let [size, decay, damping] = [0, 1, 2].map(|p| context.value_at(p, i));
                self.make(size, decay, damping, fractional);
            }
            let pre_frames = self.pre_frames_for(context.value_at(Self::PRE_DELAY, i));
            let (wet, dry) = (
                context.value_at(Self::WET, i),
                context.value_at(Self::DRY, i),
            );
            // Through the pre-delay.
            let len = self.pre.len();
            self.pre[self.pre_at] = *frame;
            let input = self.pre[(self.pre_at + len - pre_frames) % len];
            self.pre_at = (self.pre_at + 1) % len;
            // Each line's output, its length behind, between two frames
            // when the size moves.
            let mut out = [0f32; LINES];
            for (l, o) in out.iter_mut().enumerate() {
                let length = self.lengths[l];
                let whole = length as usize;
                let t = length - whole as f32;
                let near = (self.at + self.ring - whole) % self.ring;
                let a = self.lines[near][l];
                *o = if t == 0. {
                    a
                } else {
                    let b = self.lines[(near + self.ring - 1) % self.ring][l];
                    a + (b - a) * t
                };
            }
            let [left, right] = self.step(out, input);
            *frame = [dry * frame[0] + wet * left, dry * frame[1] + wet * right];
        }
    }

    /// Until it has fallen 90 dB: one and a half decays, after its
    /// pre-delay and its longest line.
    fn tail(&self) -> Duration {
        let [_, decay, ..] = self.made;
        let decay = if decay.is_nan() { 20. } else { decay };
        let first = (self.pre_frames as f32 + self.lengths[LINES - 1]) / self.rate;
        Duration::from_secs_f32(1.5 * decay + first)
    }
}
