//! Binaural sound: a bus's sound convolved with the head-related impulse
//! responses of the direction it comes from, one for each ear, so it's
//! heard round the head in headphones. The convolution is uniformly
//! partitioned (overlap-save, in blocks of [`BLOCK`] frames): fixed blocks,
//! so the same samples come out however the mixer splits its own, a block
//! late.
use super::fft::{Complex, Fft};
use crate::spatial::Vec3;
use crate::{Context, Effect, Error, Format, Parameter};
use std::sync::Arc;
use std::time::Duration;

/// Frames a partition holds, and the convolution's latency.
pub const BLOCK: usize = 128;

/// A set of head-related impulse responses: a pair (left ear, right ear)
/// for each of its directions, at its rate.
#[derive(Clone, Debug)]
pub struct HrtfSet {
    rate: u32,
    directions: Box<[Vec3]>,
    taps: usize,
    /// Each direction's responses, left then right, `taps` each.
    responses: Box<[f32]>,
}

impl HrtfSet {
    /// A set from `directions` (unit vectors, y up, -z ahead, x right) and
    /// each one's left and right responses, all of one length, at `rate`.
    pub fn new(
        rate: u32,
        directions: Vec<Vec3>,
        left: Vec<Vec<f32>>,
        right: Vec<Vec<f32>>,
    ) -> Result<HrtfSet, Error> {
        let bad = |why: &str| Err(Error::Format(format!("a set of responses {why}")));
        let taps = left.first().map_or(0, Vec::len);
        if directions.is_empty()
            || left.len() != directions.len()
            || right.len() != directions.len()
        {
            return bad("with a pair for each direction");
        }
        if taps == 0 || taps > 4_096 {
            return bad("of 1 to 4096 taps");
        }
        if left.iter().chain(&right).any(|r| r.len() != taps) {
            return bad("all of one length");
        }
        if left.iter().chain(&right).flatten().any(|s| !s.is_finite()) {
            return bad("of finite samples");
        }
        if !(1_000..=768_000).contains(&rate) {
            return bad("at a rate the mixer runs at");
        }
        let directions: Box<[Vec3]> = directions
            .into_iter()
            .map(|d| {
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if l > 1e-9 && l.is_finite() {
                    [d[0] / l, d[1] / l, d[2] / l]
                } else {
                    [0., 0., -1.]
                }
            })
            .collect();
        let responses = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| l.iter().chain(r))
            .copied()
            .collect();
        Ok(HrtfSet {
            rate,
            directions,
            taps,
            responses,
        })
    }

    /// The direction nearest `toward`.
    fn nearest(&self, toward: Vec3) -> usize {
        let mut best = (f32::MIN, 0);
        for (i, d) in self.directions.iter().enumerate() {
            let along = d[0] * toward[0] + d[1] * toward[1] + d[2] * toward[2];
            if along > best.0 {
                best = (along, i);
            }
        }
        best.1
    }

    fn pair(&self, direction: usize) -> (&[f32], &[f32]) {
        let at = direction * self.taps * 2;
        let both = &self.responses[at..at + self.taps * 2];
        both.split_at(self.taps)
    }
}

/// A direction from azimuth (degrees, 0 ahead, positive to the right) and
/// elevation (degrees up).
fn toward(azimuth: f32, elevation: f32) -> Vec3 {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [e.cos() * a.sin(), e.sin(), -e.cos() * a.cos()]
}

/// One direction's filter: each partition's spectrum of the left ear's
/// response plus i times the right's. The input is real, so one product
/// with its spectrum and one transform back give the left ear in the real
/// part and the right in the imaginary.
#[derive(Clone, Debug, Default)]
struct Filter {
    spectra: Vec<Complex>,
}

/// A bus's sound (its left and right averaged) heard binaurally from a
/// direction, through an [`HrtfSet`]: for headphones. A block
/// ([`BLOCK`] frames) late.
///
/// Parameters: [`AZIMUTH`](Self::AZIMUTH) in degrees (-180 to 180, 0
/// ahead, positive right), [`ELEVATION`](Self::ELEVATION) in degrees (-90
/// to 90). The nearest direction the set has is used; a change of it
/// crossfades from the old pair to the new over a block.
#[derive(Clone, Debug)]
pub struct Binaural {
    set: Arc<HrtfSet>,
    params: [Parameter; 2],
    fft: Option<Fft>,
    partitions: usize,
    /// The output's rate, and the set's responses at it (resampled in
    /// `prepare`).
    rate: u32,
    responses: Vec<f32>,
    taps: usize,
    /// The direction now and before, and their filters; and the azimuth
    /// and elevation it was found for, so a still one isn't sought again.
    direction: Option<usize>,
    facing: [f32; 2],
    filter: Filter,
    previous: Filter,
    /// Input frames gathered for the next block, and the last block's
    /// output, read as the next fills.
    gathered: Vec<f32>,
    ready: Vec<[f32; 2]>,
    at: usize,
    /// The last two blocks of input, transformed into the newest of the
    /// frequency-domain delay line's partitions.
    time: Vec<Complex>,
    line: Vec<Complex>,
    newest: usize,
    scratch: Vec<Complex>,
}

impl Binaural {
    pub const AZIMUTH: usize = 0;
    pub const ELEVATION: usize = 1;

    /// Through `set`, from straight ahead.
    pub fn new(set: Arc<HrtfSet>) -> Binaural {
        Binaural {
            rate: set.rate,
            set,
            params: [
                Parameter::new("azimuth", 0., -180., 180.),
                Parameter::new("elevation", 0., -90., 90.),
            ],
            fft: None,
            partitions: 0,
            responses: Vec::new(),
            taps: 0,
            direction: None,
            facing: [f32::NAN; 2],
            filter: Filter::default(),
            previous: Filter::default(),
            gathered: Vec::new(),
            ready: Vec::new(),
            at: 0,
            time: Vec::new(),
            line: Vec::new(),
            newest: 0,
            scratch: Vec::new(),
        }
    }

    /// From this direction at the start.
    pub fn facing(mut self, azimuth: f32, elevation: f32) -> Binaural {
        self.params[Self::AZIMUTH].default = azimuth.clamp(-180., 180.);
        self.params[Self::ELEVATION].default = elevation.clamp(-90., 90.);
        self
    }

    /// The partitions' spectra of `direction`'s pair into the filter.
    fn make(&mut self, direction: usize) {
        let (n, taps) = (BLOCK * 2, self.taps);
        let fft = self.fft.as_ref().expect("prepared");
        let pair = direction * taps * 2;
        let (left, right) = self.responses[pair..pair + taps * 2].split_at(taps);
        for p in 0..self.partitions {
            let spectrum = &mut self.filter.spectra[p * n..(p + 1) * n];
            spectrum.fill([0.; 2]);
            let range = (p * BLOCK).min(taps)..((p + 1) * BLOCK).min(taps);
            let part = left[range.clone()].iter().zip(&right[range]);
            for (s, (&l, &r)) in spectrum.iter_mut().zip(part) {
                *s = [l, r];
            }
            fft.run(spectrum, false);
        }
    }

    /// The input's partitions through `filter`, summed and back to time,
    /// into `out`: the left ear in the real parts, the right in the
    /// imaginary.
    fn convolve(&self, filter: &Filter, out: &mut [Complex]) {
        let n = BLOCK * 2;
        out.fill([0.; 2]);
        for p in 0..self.partitions {
            let x = (self.newest + self.partitions - p) % self.partitions;
            let spectrum = &self.line[x * n..(x + 1) * n];
            let h = &filter.spectra[p * n..(p + 1) * n];
            for ((o, a), b) in out.iter_mut().zip(spectrum).zip(h) {
                o[0] += a[0] * b[0] - a[1] * b[1];
                o[1] += a[0] * b[1] + a[1] * b[0];
            }
        }
        self.fft.as_ref().expect("prepared").run(out, true);
    }

    /// One block of `BLOCK` gathered frames through the convolution, the
    /// last block's output replaced.
    fn block(&mut self, facing: [f32; 2]) {
        let n = BLOCK * 2;
        let direction = match self.direction {
            Some(direction) if facing == self.facing => direction,
            _ => {
                self.facing = facing;
                self.set.nearest(toward(facing[0], facing[1]))
            }
        };
        let changed = self.direction.is_some_and(|d| d != direction);
        if self.direction != Some(direction) {
            if changed {
                std::mem::swap(&mut self.filter, &mut self.previous);
            }
            self.make(direction);
            self.direction = Some(direction);
        }
        // The last two blocks of input, the newer second: transformed into
        // the line's next partition.
        self.time.copy_within(BLOCK..n, 0);
        for (t, &x) in self.time[BLOCK..].iter_mut().zip(&self.gathered) {
            *t = [x, 0.];
        }
        self.newest = (self.newest + 1) % self.partitions;
        let fft = self.fft.as_ref().expect("prepared");
        let slot = &mut self.line[self.newest * n..(self.newest + 1) * n];
        slot.copy_from_slice(&self.time);
        fft.run(slot, false);
        // The partitions' products summed and back to time: the newer half
        // is the block's output.
        let scale = 1. / n as f32;
        let mut out = std::mem::take(&mut self.scratch);
        self.convolve(&self.filter, &mut out);
        for (frame, z) in self.ready.iter_mut().zip(&out[BLOCK..]) {
            *frame = [z[0] * scale, z[1] * scale];
        }
        if changed {
            // The old pair's output too, faded out under the new one's.
            self.convolve(&self.previous, &mut out);
            for (k, (frame, z)) in self.ready.iter_mut().zip(&out[BLOCK..]).enumerate() {
                let t = (k + 1) as f32 / BLOCK as f32;
                let old = [z[0] * scale, z[1] * scale];
                *frame = [
                    old[0] + (frame[0] - old[0]) * t,
                    old[1] + (frame[1] - old[1]) * t,
                ];
            }
        }
        self.scratch = out;
    }
}

impl Effect for Binaural {
    fn parameters(&self) -> &[Parameter] {
        &self.params
    }

    fn prepare(&mut self, format: Format, _max_block: usize) {
        // The set's responses at the output's rate, keeping their gain:
        // read between their samples, scaled by the ratio, going up; each
        // sample spread over its two nearest, going down (the first's
        // transpose, a triangle wide enough to take what would alias).
        self.rate = format.rate.max(1);
        let ratio = f64::from(self.set.rate) / f64::from(self.rate);
        let taps = if self.set.rate == self.rate {
            self.set.taps
        } else {
            ((self.set.taps - 1) as f64 / ratio).floor() as usize + 2
        };
        self.taps = taps;
        self.responses = vec![0.; self.set.directions.len() * taps * 2];
        for (d, out) in self.responses.chunks_exact_mut(taps * 2).enumerate() {
            let (l, r) = self.set.pair(d);
            for (response, out) in [l, r].into_iter().zip(out.chunks_exact_mut(taps)) {
                if ratio == 1. {
                    out.copy_from_slice(response);
                } else if ratio < 1. {
                    for (i, o) in out.iter_mut().enumerate() {
                        let at = i as f64 * ratio;
                        let (j, t) = (at as usize, (at - at.floor()) as f32);
                        let a = response.get(j).copied().unwrap_or(0.);
                        let b = response.get(j + 1).copied().unwrap_or(0.);
                        *o = (a + (b - a) * t) * ratio as f32;
                    }
                } else {
                    for (j, &x) in response.iter().enumerate() {
                        let at = j as f64 / ratio;
                        let (i, t) = (at as usize, (at - at.floor()) as f32);
                        out[i] += x * (1. - t);
                        out[i + 1] += x * t;
                    }
                }
            }
        }
        let n = BLOCK * 2;
        self.fft = Some(Fft::new(n));
        self.partitions = taps.div_ceil(BLOCK);
        let spectra = || vec![[0f32; 2]; self.partitions * n];
        self.filter = Filter { spectra: spectra() };
        self.previous = Filter { spectra: spectra() };
        self.line = spectra();
        self.time = vec![[0.; 2]; n];
        self.scratch = vec![[0.; 2]; n];
        self.gathered = vec![0.; BLOCK];
        self.ready = vec![[0.; 2]; BLOCK];
        self.at = 0;
        self.newest = 0;
        self.direction = None;
        self.facing = [f32::NAN; 2];
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        for (i, frame) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            self.gathered[self.at] = (frame[0] + frame[1]) * 0.5;
            *frame = self.ready[self.at];
            self.at += 1;
            if self.at == BLOCK {
                self.at = 0;
                self.block([context.value_at(0, i), context.value_at(1, i)]);
            }
        }
    }

    /// The responses' length, and the block it's late by.
    fn tail(&self) -> Duration {
        Duration::from_secs_f64(
            self.set.taps as f64 / f64::from(self.set.rate)
                + (2 * BLOCK) as f64 / f64::from(self.rate),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_direction_is_the_one_most_along_it() {
        let set = HrtfSet::new(
            48_000,
            vec![[0., 0., -1.], [1., 0., 0.], [-1., 0., 0.], [0., 1., 0.]],
            vec![vec![1.]; 4],
            vec![vec![1.]; 4],
        )
        .unwrap();
        assert_eq!(set.nearest(toward(0., 0.)), 0);
        assert_eq!(set.nearest(toward(80., 0.)), 1);
        assert_eq!(set.nearest(toward(-100., 10.)), 2);
        assert_eq!(set.nearest(toward(0., 70.)), 3);
    }

    #[test]
    fn a_set_that_can_t_be_used_is_an_error() {
        let one = || vec![vec![0.5f32; 8]];
        assert!(HrtfSet::new(48_000, vec![], vec![], vec![]).is_err());
        assert!(HrtfSet::new(48_000, vec![[0., 0., -1.]], one(), vec![vec![0.5; 7]]).is_err());
        assert!(HrtfSet::new(48_000, vec![[0., 0., -1.]], vec![vec![f32::NAN; 8]], one()).is_err());
        assert!(HrtfSet::new(10, vec![[0., 0., -1.]], one(), one()).is_err());
        assert!(HrtfSet::new(48_000, vec![[0., 0., -1.]], one(), one()).is_ok());
    }
}
