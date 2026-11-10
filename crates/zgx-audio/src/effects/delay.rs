//! An echo: a line of frames as long as its longest time, read that time
//! behind where it's written, fed back into itself.
use crate::{Context, Effect, Format, Parameter};
use std::time::Duration;

/// An echo: the sound again after [`TIME`](Self::TIME), and again, each
/// time [`FEEDBACK`](Self::FEEDBACK) as loud. Its line is as long as the
/// longest time it was made for, made in `prepare`.
///
/// Parameters: [`TIME`](Self::TIME) in seconds (a frame to the longest);
/// [`FEEDBACK`](Self::FEEDBACK), each echo's gain over the last (0 to
/// 0.98); [`WET`](Self::WET), the echoes' gain (0 to 1); [`DRY`](Self::DRY),
/// the sound's own (0 to 1). A time that moves glides, reading between
/// frames, as tape does: no click.
///
/// With [`DRY`](Self::DRY) 1 and [`WET`](Self::WET) the same as
/// [`FEEDBACK`](Self::FEEDBACK) it's a feedback comb, `y[n] = x[n] + g
/// y[n - d]`: miniaudio's `ma_delay`.
#[derive(Clone, Debug)]
pub struct Delay {
    params: [Parameter; 4],
    /// The longest time, in seconds.
    longest: f32,
    /// Frames, interleaved, as a ring: written at `at`.
    line: Vec<f32>,
    frames: usize,
    at: usize,
    rate: f32,
    /// The time and feedback it last ran with: what its tail follows.
    now: [f32; 2],
}

impl Delay {
    pub const TIME: usize = 0;
    pub const FEEDBACK: usize = 1;
    pub const WET: usize = 2;
    pub const DRY: usize = 3;

    /// An echo `time` seconds later, half as loud each time, its time able
    /// to reach `longest` seconds (at most 10).
    pub fn new(time: f32, longest: f32) -> Delay {
        let longest = longest.clamp(0.001, 10.);
        Delay {
            params: [
                Parameter::new("time", time.clamp(0., longest), 0., longest),
                Parameter::new("feedback", 0.5, 0., 0.98),
                Parameter::new("wet", 0.5, 0., 1.),
                Parameter::new("dry", 1., 0., 1.),
            ],
            longest,
            line: Vec::new(),
            frames: 0,
            at: 0,
            rate: 48_000.,
            now: [time.clamp(0., longest), 0.5],
        }
    }

    /// Each echo's gain over the one before.
    pub fn feedback(mut self, gain: f32) -> Delay {
        self.params[Self::FEEDBACK].default = gain.clamp(0., 0.98);
        self
    }

    /// The echoes' gain, and the sound's own.
    pub fn mix(mut self, wet: f32, dry: f32) -> Delay {
        self.params[Self::WET].default = wet.clamp(0., 1.);
        self.params[Self::DRY].default = dry.clamp(0., 1.);
        self
    }

    /// Frames behind: at least one, at most the line holds. Within a
    /// thousandth of a whole frame it's that frame: a time meant as whole
    /// frames (half a millisecond is 24) isn't read between two for a
    /// float's rounding.
    fn behind(&self, seconds: f32) -> f32 {
        let frames = seconds * self.rate;
        let whole = frames.round();
        let frames = if (frames - whole).abs() < 1e-3 {
            whole
        } else {
            frames
        };
        frames.clamp(1., (self.frames - 1) as f32)
    }
}

impl Effect for Delay {
    fn parameters(&self) -> &[Parameter] {
        &self.params
    }

    fn prepare(&mut self, format: Format, max_block: usize) {
        self.rate = format.rate as f32;
        // The longest time, a frame either side to read between, and a
        // block more: the frames a block reads are then never the ones it
        // writes, so it runs whole, in one pass that vectorises.
        self.frames = (self.longest * self.rate).ceil() as usize + 2 + max_block;
        self.line = vec![0.; self.frames * 2];
        self.at = 0;
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let frames = block.as_chunks_mut::<2>().0;
        let [time, feedback, wet, dry] = [0, 1, 2, 3].map(|p| context.value(p));
        let last = frames.len().saturating_sub(1);
        self.now = [context.value_at(0, last), context.value_at(1, last)];
        let steady = (0..4).all(|p| context.values(p).is_none());
        let behind = self.behind(time);
        if steady && behind.fract() == 0. {
            self.steady(frames, behind as usize, feedback, wet, dry);
        } else {
            for (i, frame) in frames.iter_mut().enumerate() {
                let [behind, feedback, wet, dry] = [
                    self.behind(context.value_at(0, i)),
                    context.value_at(1, i),
                    context.value_at(2, i),
                    context.value_at(3, i),
                ];
                // Between two frames: a straight line, exact on a frame.
                let whole = behind as usize;
                let t = behind - whole as f32;
                let near = (self.at + self.frames - whole) % self.frames;
                let far = (near + self.frames - 1) % self.frames;
                for (c, x) in frame.iter_mut().enumerate() {
                    let (a, b) = (self.line[near * 2 + c], self.line[far * 2 + c]);
                    let echo = a + (b - a) * t;
                    self.line[self.at * 2 + c] = *x + feedback * echo;
                    *x = dry * *x + wet * echo;
                }
                self.at = (self.at + 1) % self.frames;
            }
        }
    }

    /// Until its echoes have fallen 90 dB: each is `feedback` times the
    /// last, so 4.5 / -log10(feedback) of them, a time apart.
    fn tail(&self) -> Duration {
        let [time, feedback] = self.now;
        let echoes = if feedback > 0. {
            4.5 / -crate::math::log10f(feedback)
        } else {
            0.
        };
        Duration::from_secs_f32(time * (1. + echoes))
    }
}

impl Delay {
    /// A whole number of frames behind, nothing moving: the block in runs
    /// no longer than the delay and within the ring, so no frame is read
    /// that the run writes, and each run vectorises.
    fn steady(
        &mut self,
        frames: &mut [[f32; 2]],
        behind: usize,
        feedback: f32,
        wet: f32,
        dry: f32,
    ) {
        let mut done = 0;
        while done < frames.len() {
            let from = (self.at + self.frames - behind) % self.frames;
            // Short of the ring's end for both, and when the read has gone
            // round past the write, short of it.
            let ahead = if from > self.at {
                from - self.at
            } else {
                usize::MAX
            };
            let n = (frames.len() - done)
                .min(behind)
                .min(self.frames - self.at)
                .min(self.frames - from)
                .min(ahead);
            let run = &mut frames[done..done + n];
            let (reads, writes) = if from < self.at {
                let (low, high) = self.line.split_at_mut(self.at * 2);
                (&low[from * 2..(from + n) * 2], &mut high[..n * 2])
            } else {
                let (low, high) = self.line.split_at_mut(from * 2);
                (&high[..n * 2], &mut low[self.at * 2..(self.at + n) * 2])
            };
            echo(
                run,
                reads.as_chunks::<2>().0,
                writes.as_chunks_mut::<2>().0,
                [feedback, wet, dry],
            );
            self.at = (self.at + n) % self.frames;
            done += n;
        }
    }
}

/// A run of frames through the line: each written to it with its echo fed
/// back, and heard with its echo. A function of its own, so the compiler
/// knows the three slices don't overlap and runs it four frames at a time
/// without checking.
#[inline(never)]
fn echo(
    frames: &mut [[f32; 2]],
    echoes: &[[f32; 2]],
    line: &mut [[f32; 2]],
    [feedback, wet, dry]: [f32; 3],
) {
    for ((frame, echo), slot) in frames.iter_mut().zip(echoes).zip(line.iter_mut()) {
        let x = *frame;
        *slot = [x[0] + feedback * echo[0], x[1] + feedback * echo[1]];
        *frame = [dry * x[0] + wet * echo[0], dry * x[1] + wet * echo[1]];
    }
}
