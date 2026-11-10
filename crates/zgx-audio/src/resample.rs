//! Reading a sound at any rate: a voice asks for frames at the output's
//! rate and gets them here, whatever the sound's own rate and pitch. The
//! interpolators live behind [`Resampler`]; voices don't see them.
use crate::sound::SoundData;

/// Where a voice is in its sound: whole frames in the high 32 bits, a
/// fraction of one in the low.
pub(crate) type Position = u64;
/// One frame, as a [`Position`] or a step.
pub(crate) const ONE: u64 = 1 << 32;

/// How a voice reads between a sound's frames, when its rate or pitch
/// isn't the output's. At the same rate and pitch every kind reads the
/// frames as they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    /// A straight line between two frames: cheapest, and a little dull (a
    /// tone near the top of the sound's band comes out a few dB down).
    Linear,
    /// A cubic through four frames (Catmull-Rom): flatter to near the top
    /// of the band, for a few instructions more a frame.
    #[default]
    Cubic,
}

/// The frames a voice plays: up to `end`, then from `start` again if it
/// loops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub start: u64,
    pub end: u64,
    pub looping: bool,
}

/// How far a voice moves through its sound for each output frame.
#[derive(Clone, Copy)]
pub(crate) enum Step<'a> {
    Fixed(u64),
    /// One step per output frame, while its pitch moves.
    PerFrame(&'a [u64]),
}

/// A voice's interpolator and what it keeps between blocks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resampler {
    interpolation: Interpolation,
    /// Gone round its loop at least once: the frames before the loop's
    /// start are then its end's.
    looped: bool,
}

impl Resampler {
    pub fn new(interpolation: Interpolation) -> Resampler {
        Resampler {
            interpolation,
            looped: false,
        }
    }

    /// Frames of `sound` from `at` on, one per slot of `left` (and of
    /// `right`, for a sound of two channels; one channel's go to `left`
    /// alone). Returns how many were written: fewer than `left.len()` means
    /// the sound ended.
    pub fn render(
        &mut self,
        sound: &SoundData,
        region: Region,
        at: &mut Position,
        step: Step<'_>,
        left: &mut [f32],
        right: &mut [f32],
    ) -> usize {
        let out = [left, right];
        let samples = &sound.samples;
        let looped = &mut self.looped;
        match (sound.channels, self.interpolation) {
            (1, Interpolation::Linear) => run::<1, Linear>(samples, region, looped, at, step, out),
            (_, Interpolation::Linear) => run::<2, Linear>(samples, region, looped, at, step, out),
            (1, Interpolation::Cubic) => run::<1, Cubic>(samples, region, looped, at, step, out),
            (_, Interpolation::Cubic) => run::<2, Cubic>(samples, region, looped, at, step, out),
        }
    }
}

/// An interpolator: it reads `BEHIND` frames before a position's and
/// `AHEAD` after.
trait Kernel {
    const BEHIND: usize;
    const AHEAD: usize;
    /// The value at `t` (0 to 1) past the frame `BEHIND` into `taps`,
    /// which holds one channel's frames `C` apart.
    fn at<const C: usize>(taps: &[f32], t: f32) -> f32;
    /// Both channels of a stereo sound: `taps` holds `BEHIND + 1 + AHEAD`
    /// frames (and zeros after).
    fn stereo(taps: &[f32; 8], t: f32) -> [f32; 2];

    /// Frames from `position` on at a fixed `step`, every one of whose taps
    /// is in `samples`, one per slot of `left` (and `right`). Returns the
    /// position after.
    #[inline(always)]
    fn span<const C: usize>(
        samples: &[f32],
        mut position: Position,
        step: u64,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Position {
        let taps = Self::BEHIND + 1 + Self::AHEAD;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let first = ((position >> 32) as usize - Self::BEHIND) * C;
            let t = fraction(position);
            if C == 2 {
                let mut window = [0f32; 8];
                window[..taps * 2].copy_from_slice(&samples[first..first + taps * 2]);
                [*l, *r] = Self::stereo(&window, t);
            } else {
                *l = Self::at::<1>(&samples[first..first + taps], t);
            }
            position += step;
        }
        position
    }
}

struct Linear;
impl Kernel for Linear {
    const BEHIND: usize = 0;
    const AHEAD: usize = 1;
    #[inline(always)]
    fn at<const C: usize>(taps: &[f32], t: f32) -> f32 {
        let (a, b) = (taps[0], taps[C]);
        a + (b - a) * t
    }
    #[inline(always)]
    fn stereo(taps: &[f32; 8], t: f32) -> [f32; 2] {
        [Self::at::<2>(taps, t), Self::at::<2>(&taps[1..], t)]
    }
}

struct Cubic;
impl Kernel for Cubic {
    const BEHIND: usize = 1;
    const AHEAD: usize = 2;
    #[inline(always)]
    fn at<const C: usize>(taps: &[f32], t: f32) -> f32 {
        cubic(taps[0], taps[C], taps[2 * C], taps[3 * C], t)
    }
    #[inline(always)]
    fn stereo(taps: &[f32; 8], t: f32) -> [f32; 2] {
        // Both channels at once, as pairs: the compiler keeps each pair in
        // one register.
        let [b0, b1, n0, n1, x0, x1, a0, a1] = *taps;
        [cubic(b0, n0, x0, a0, t), cubic(b1, n1, x1, a1, t)]
    }

    /// In runs of 64 frames: first each frame's taps are gathered into a
    /// row per tap, then the cubic runs down the rows, where it vectorises
    /// (the same operations in the same order: the same results).
    fn span<const C: usize>(
        samples: &[f32],
        mut position: Position,
        step: u64,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Position {
        const RUN: usize = 64;
        for (left, right) in left.chunks_mut(RUN).zip(right.chunks_mut(RUN)) {
            let n = left.len();
            let mut rows = [[[0f32; RUN]; 4]; C];
            let mut ts = [0f32; RUN];
            for (j, t) in ts[..n].iter_mut().enumerate() {
                let first = ((position >> 32) as usize - 1) * C;
                let window = &samples[first..first + 4 * C];
                for (channel, rows) in rows.iter_mut().enumerate() {
                    for (k, row) in rows.iter_mut().enumerate() {
                        row[j] = window[k * C + channel];
                    }
                }
                *t = fraction(position);
                position += step;
            }
            for (channel, out) in [&mut *left, &mut *right].into_iter().take(C).enumerate() {
                let [before, now, next, after] = &rows[channel];
                for j in 0..n {
                    out[j] = cubic(before[j], now[j], next[j], after[j], ts[j]);
                }
            }
        }
        position
    }
}

#[inline(always)]
fn cubic(before: f32, now: f32, next: f32, after: f32, t: f32) -> f32 {
    let c1 = (next - before) * 0.5;
    let c2 = before - now * 2.5 + next * 2. - after * 0.5;
    let c3 = (after - before) * 0.5 + (now - next) * 1.5;
    ((c3 * t + c2) * t + c1) * t + now
}

#[inline(always)]
fn fraction(at: Position) -> f32 {
    (at as u32) as f32 * (1. / ONE as f32)
}

/// One channel's frame `index` as a voice in `region` hears it: past the
/// end, the loop's start (or silence); before a looped voice's start, its
/// end; before the sound, silence.
fn tap<const C: usize>(
    samples: &[f32],
    region: Region,
    looped: bool,
    index: i64,
    channel: usize,
) -> f32 {
    let (start, end) = (region.start as i64, region.end as i64);
    let looping = region.looping && end > start;
    let mut index = index;
    if index >= end {
        if !looping {
            return 0.;
        }
        index = start + (index - end) % (end - start);
    } else if looped && looping && index < start {
        index = end - 1 - (start - 1 - index) % (end - start);
    }
    if index < 0 {
        return 0.;
    }
    samples[index as usize * C + channel]
}

fn run<const C: usize, K: Kernel>(
    samples: &[f32],
    region: Region,
    looped: &mut bool,
    at: &mut Position,
    step: Step<'_>,
    [left, right]: [&mut [f32]; 2],
) -> usize {
    let wanted = left.len();
    let (start, end) = (region.start, region.end);
    let looping = region.looping && end > start;
    let taps = K::BEHIND + 1 + K::AHEAD;
    let step_at = |i: usize| match step {
        Step::Fixed(step) => step,
        Step::PerFrame(steps) => steps[i],
    };
    let mut i = 0;
    while i < wanted {
        if *at >> 32 >= end {
            if !looping {
                break;
            }
            *at = (start << 32) + (*at - (end << 32)) % ((end - start) << 32);
            *looped = true;
        }
        let frame = (*at >> 32) as usize;
        // Frames read in the sound itself: from `low` (after a loop, the
        // frames before its start aren't the ones before), up to `limit`.
        let low = (if *looped { start } else { 0 }) + K::BEHIND as u64;
        let limit = end.saturating_sub(K::AHEAD as u64) << 32;
        if let Step::Fixed(ONE) = step
            && *at as u32 == 0
        {
            // The sound's own rate, on its frames: a copy.
            let count = ((end as usize) - frame).min(wanted - i);
            let from = &samples[frame * C..(frame + count) * C];
            let (left, right) = (&mut left[i..i + count], &mut right[i..i + count]);
            if C == 1 {
                left.copy_from_slice(from);
            } else {
                for ((l, r), pair) in left
                    .iter_mut()
                    .zip(right.iter_mut())
                    .zip(from.as_chunks::<2>().0)
                {
                    [*l, *r] = *pair;
                }
            }
            *at += (count as u64) << 32;
            i += count;
        } else if frame as u64 >= low && *at < limit {
            match step {
                Step::Fixed(step) => {
                    // The least in u64, then a usize: on a 32-bit target the
                    // frames to the limit may not fit one.
                    let count = (limit - *at).div_ceil(step).min((wanted - i) as u64) as usize;
                    *at = K::span::<C>(
                        samples,
                        *at,
                        step,
                        &mut left[i..i + count],
                        &mut right[i..i + count],
                    );
                    i += count;
                }
                Step::PerFrame(steps) => {
                    while i < wanted && *at < limit {
                        let first = ((*at >> 32) as usize - K::BEHIND) * C;
                        let window = &samples[first..first + taps * C];
                        let t = fraction(*at);
                        left[i] = K::at::<C>(window, t);
                        if C == 2 {
                            right[i] = K::at::<C>(&window[1..], t);
                        }
                        *at += steps[i];
                        i += 1;
                    }
                }
            }
        } else {
            // Near the region's edges: each tap found on its own.
            let mut window = [0f32; 8];
            let first = frame as i64 - K::BEHIND as i64;
            for k in 0..taps {
                for channel in 0..C {
                    window[k * C + channel] =
                        tap::<C>(samples, region, *looped, first + k as i64, channel);
                }
            }
            let t = fraction(*at);
            left[i] = K::at::<C>(&window, t);
            if C == 2 {
                right[i] = K::at::<C>(&window[1..], t);
            }
            *at += step_at(i);
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(samples: &[f32], channels: usize) -> SoundData {
        SoundData {
            samples: samples.into(),
            channels,
            rate: 48_000,
        }
    }

    fn region(start: u64, end: u64, looping: bool) -> Region {
        Region {
            start,
            end,
            looping,
        }
    }

    fn read(
        resampler: &mut Resampler,
        sound: &SoundData,
        region: Region,
        at: &mut Position,
        step: u64,
        n: usize,
    ) -> (Vec<f32>, Vec<f32>, usize) {
        let (mut l, mut r) = (vec![0.; n], vec![0.; n]);
        let got = resampler.render(sound, region, at, Step::Fixed(step), &mut l, &mut r);
        (l, r, got)
    }

    #[test]
    fn at_its_own_rate_a_sound_is_copied_then_ends() {
        for kind in [Interpolation::Linear, Interpolation::Cubic] {
            let stereo = sound(&[1., -1., 2., -2., 3., -3.], 2);
            let mut at = 0;
            let (l, r, got) = read(
                &mut Resampler::new(kind),
                &stereo,
                region(0, 3, false),
                &mut at,
                ONE,
                5,
            );
            assert_eq!(got, 3);
            assert_eq!(
                (&l[..3], &r[..3]),
                (&[1., 2., 3.][..], &[-1., -2., -3.][..])
            );
        }
    }

    #[test]
    fn between_frames_linear_draws_straight_lines_and_loops_round_to_the_start() {
        let mono = sound(&[0., 1., 2., 3.], 1);
        let mut at = 0;
        let mut linear = Resampler::new(Interpolation::Linear);
        let (l, _, got) = read(&mut linear, &mono, region(1, 4, true), &mut at, ONE / 2, 10);
        assert_eq!(got, 10);
        // 3 runs on to the loop's start (1), then round again.
        assert_eq!(l, [0., 0.5, 1., 1.5, 2., 2.5, 3., 2., 1., 1.5]);
    }

    #[test]
    fn a_cubic_passes_through_every_frame_and_follows_a_line_exactly() {
        let line: Vec<f32> = (0..64).map(|i| i as f32).collect();
        let mono = sound(&line, 1);
        let mut at = 0;
        let mut cubic = Resampler::new(Interpolation::Cubic);
        let (l, _, _) = read(
            &mut cubic,
            &mono,
            region(0, 64, false),
            &mut at,
            ONE / 4,
            200,
        );
        // Away from the ends, where its taps are the line's own.
        for (i, &v) in l.iter().enumerate().skip(4).take(240 - 12) {
            assert!((v - i as f32 / 4.).abs() < 1e-4, "{i}: {v}");
        }
    }

    #[test]
    fn a_looped_cubic_reads_round_the_loop_seamlessly() {
        // One cycle of a sine, looped: read at a fraction, it must be the
        // same as reading a long sine.
        let cycle = 32;
        let sine: Vec<f32> = (0..cycle * 4)
            .map(|i| (i as f32 * std::f32::consts::TAU / cycle as f32).sin())
            .collect();
        let (looped, long) = (sound(&sine[..cycle], 1), sound(&sine, 1));
        let step = (ONE as f64 * 0.37) as u64;
        let (mut a, mut b) = (0, 0);
        let (round, _, _) = read(
            &mut Resampler::new(Interpolation::Cubic),
            &looped,
            region(0, cycle as u64, true),
            &mut a,
            step,
            300,
        );
        let (straight, _, _) = read(
            &mut Resampler::new(Interpolation::Cubic),
            &long,
            region(0, 4 * cycle as u64, false),
            &mut b,
            step,
            300,
        );
        // From the second cycle on, the loop's frames before its start are its end's.
        for i in (cycle as f64 / 0.37) as usize + 4..300 {
            assert!(
                (round[i] - straight[i]).abs() < 1e-5,
                "{i}: {} {}",
                round[i],
                straight[i]
            );
        }
    }

    #[test]
    fn a_block_split_anywhere_reads_the_same() {
        for kind in [Interpolation::Linear, Interpolation::Cubic] {
            let samples: Vec<f32> = (0..200).map(|i| (i as f32 * 0.37).sin()).collect();
            let stereo = sound(&samples, 2);
            let step = (ONE as f64 * 1.37) as u64;
            let mut at = 0;
            let (whole, _, _) = read(
                &mut Resampler::new(kind),
                &stereo,
                region(10, 100, true),
                &mut at,
                step,
                300,
            );
            let mut at = 0;
            let mut parts = Vec::new();
            let mut resampler = Resampler::new(kind);
            for n in [1, 7, 64, 100, 128] {
                parts.extend(
                    read(
                        &mut resampler,
                        &stereo,
                        region(10, 100, true),
                        &mut at,
                        step,
                        n,
                    )
                    .0,
                );
            }
            assert_eq!(whole, parts);
        }
    }

    #[test]
    fn per_frame_steps_match_a_fixed_one() {
        for kind in [Interpolation::Linear, Interpolation::Cubic] {
            let mono = sound(&(0..64).map(|i| (i as f32).sqrt()).collect::<Vec<_>>(), 1);
            let step = ONE * 3 / 4;
            let (mut a, mut b) = (0, 0);
            let (fixed, _, _) = read(
                &mut Resampler::new(kind),
                &mono,
                region(0, 64, true),
                &mut a,
                step,
                100,
            );
            let (mut varied, mut r) = (vec![0.; 100], vec![0.; 100]);
            Resampler::new(kind).render(
                &mono,
                region(0, 64, true),
                &mut b,
                Step::PerFrame(&[step; 100]),
                &mut varied,
                &mut r,
            );
            assert_eq!((fixed, a), (varied, b));
        }
    }
}
