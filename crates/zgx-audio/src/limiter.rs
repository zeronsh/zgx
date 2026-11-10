//! The look-ahead limiter on the master bus: nothing leaves above its
//! ceiling, and what's below passes untouched, a fixed delay late.
//!
//! Each sample asks for the gain that would bring it to the ceiling. The
//! gain applied is the average, over the look-ahead, of the least gain any
//! sample in the window behind it asked for (eased back up by the
//! release): every value averaged is at most the gain the sample leaving
//! the delay asked for, so it never passes the ceiling, and the attack is a
//! straight ramp as long as the look-ahead.

/// Gains held as integers of this many steps to one, so the window's sum
/// is exact however long it runs.
const UNIT: u32 = 1 << 24;

pub(crate) struct Limiter {
    ceiling: f32,
    /// Look-ahead, in frames: the delay, and the attack.
    look: usize,
    delay: Box<[[f32; 2]]>,
    /// Gains averaged, as [`UNIT`]s, a ring of `look`.
    window: Box<[u32]>,
    sum: u64,
    /// The next slot of `delay` and `window`.
    at: usize,
    /// Least gains asked for in the last `look + 1` samples, smallest first:
    /// (sample, gain). Gains of 1 aren't kept: an empty queue means 1.
    least: Box<[(u64, f32)]>,
    least_head: usize,
    least_len: usize,
    /// One less the gain asked for after the release, sample by sample:
    /// held as the reduction, whose small values a float keeps exactly.
    reduction: f32,
    /// What the reduction is multiplied by each sample as it releases.
    decay: f32,
    sample: u64,
    /// Samples since the delay last held anything but silence.
    quiet: usize,
    /// Samples the last clamp brought back by more than rounding: none,
    /// if the gain did its work. Tests read it, so a clamp that hides a
    /// broken gain fails them.
    #[cfg(test)]
    clipped: u64,
}

impl Limiter {
    pub fn new(rate: u32, ceiling: f32) -> Limiter {
        let look = ((rate as f32 * 0.0015).round() as usize).max(1);
        Limiter {
            ceiling,
            look,
            delay: vec![[0.; 2]; look].into(),
            window: vec![UNIT; look].into(),
            sum: u64::from(UNIT) * look as u64,
            at: 0,
            least: vec![(0, 1.); look + 1].into(),
            least_head: 0,
            least_len: 0,
            reduction: 0.,
            // 80 ms to come back to 1/e of the reduction.
            decay: (-1. / (0.08 * rate as f32)).exp(),
            sample: 0,
            quiet: look,
            #[cfg(test)]
            clipped: 0,
        }
    }

    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// Whether it holds nothing but silence and isn't limiting.
    pub fn silent(&self) -> bool {
        self.quiet >= self.look && self.at_rest()
    }

    /// The frames its output lags its input.
    pub fn latency(&self) -> usize {
        self.look
    }

    fn at_rest(&self) -> bool {
        self.least_len == 0
            && self.reduction == 0.
            && self.sum == u64::from(UNIT) * self.look as u64
    }

    /// Limits `left` and `right` in place, `look` frames late.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let peak = left
            .iter()
            .chain(right.iter())
            .fold(0f32, |peak, s| peak.max(s.abs()));
        if peak <= self.ceiling && self.at_rest() {
            // Below the ceiling and nothing being limited: only the delay.
            if peak == 0. && self.quiet >= self.look {
                self.sample += left.len() as u64;
                return;
            }
            self.quiet = if peak == 0. {
                self.quiet + left.len()
            } else {
                0
            };
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let slot = &mut self.delay[self.at];
                (*l, *r, slot[0], slot[1]) = (slot[0], slot[1], *l, *r);
                self.at = if self.at + 1 == self.look {
                    0
                } else {
                    self.at + 1
                };
            }
            self.sample += left.len() as u64;
            return;
        }
        self.quiet = 0;
        let capacity = self.least.len();
        let scale = 1. / (f64::from(UNIT) * self.look as f64);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let n = self.sample;
            // Gains asked for before the window first: then what's kept is
            // within the last `look + 1` samples, which the ring holds.
            while self.least_len > 0 && self.least[self.least_head].0 + (self.look as u64) < n {
                self.least_head = (self.least_head + 1) % capacity;
                self.least_len -= 1;
            }
            let loudest = l.abs().max(r.abs());
            if loudest > self.ceiling {
                let asked = self.ceiling / loudest;
                while self.least_len > 0 {
                    let back = (self.least_head + self.least_len - 1) % capacity;
                    if self.least[back].1 < asked {
                        break;
                    }
                    self.least_len -= 1;
                }
                self.least[(self.least_head + self.least_len) % capacity] = (n, asked);
                self.least_len += 1;
            }
            let least = if self.least_len > 0 {
                self.least[self.least_head].1
            } else {
                1.
            };
            let mut reduction = self.reduction * self.decay;
            if reduction < 1e-7 {
                reduction = 0.;
            }
            self.reduction = reduction.max(1. - least);
            // Rounded down, so the average never rounds above what was asked.
            let gain = ((1. - f64::from(self.reduction)) * f64::from(UNIT)) as u32;
            self.sum = self.sum + u64::from(gain) - u64::from(self.window[self.at]);
            self.window[self.at] = gain;
            let applied = (self.sum as f64 * scale) as f32;
            let slot = &mut self.delay[self.at];
            let (out_l, out_r) = (slot[0] * applied, slot[1] * applied);
            (slot[0], slot[1]) = (*l, *r);
            // Rounding in the last bit is the only way past the ceiling.
            #[cfg(test)]
            {
                let over = out_l.abs().max(out_r.abs()) - self.ceiling;
                if over > self.ceiling * 4. * f32::EPSILON {
                    self.clipped += 1;
                }
            }
            *l = out_l.clamp(-self.ceiling, self.ceiling);
            *r = out_r.clamp(-self.ceiling, self.ceiling);
            self.at = if self.at + 1 == self.look {
                0
            } else {
                self.at + 1
            };
            self.sample += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_signals_pass_unchanged_a_look_ahead_late() {
        let mut limiter = Limiter::new(48_000, 1.);
        let look = limiter.latency();
        assert_eq!(look, 72);
        let input: Vec<f32> = (0..300).map(|i| (i as f32 * 0.1).sin() * 0.9).collect();
        let (mut l, mut r) = (input.clone(), input.clone());
        limiter.process(&mut l, &mut r);
        assert_eq!(&l[look..], &input[..300 - look]);
        assert!(l[..look].iter().all(|&s| s == 0.));
    }

    #[test]
    fn nothing_leaves_above_the_ceiling_and_the_gain_comes_back() {
        let mut limiter = Limiter::new(48_000, 0.5);
        let n = 96_000;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                let loud = if (1000..1400).contains(&i) { 4. } else { 0.25 };
                (i as f32 * 0.05).sin() * loud
            })
            .collect();
        let (mut l, mut r) = (input.clone(), input.clone());
        for (l, r) in l.chunks_mut(100).zip(r.chunks_mut(100)) {
            limiter.process(l, r);
        }
        assert!(l.iter().all(|s| s.abs() <= 0.5));
        assert_eq!(limiter.clipped, 0, "the gain let samples past the ceiling");
        let look = limiter.latency();
        // The burst is brought down to the ceiling, not below it by much.
        let burst = l[1000 + look..1400 + look]
            .iter()
            .fold(0f32, |p, s| p.max(s.abs()));
        assert!(burst > 0.49, "{burst}");
        // Two seconds later the quiet signal is back as it was.
        assert_eq!(&l[n - 100..], &input[n - 100 - look..n - look]);
        assert!(limiter.at_rest());
    }

    #[test]
    fn a_loud_low_tone_is_brought_to_the_ceiling_by_the_gain_alone() {
        // Below about 160 Hz a falling half-cycle asks for more gains than
        // the window holds: each must be kept, or the gain lets samples
        // past the ceiling and only the final clamp hides it.
        for hz in [30f32, 80., 150., 400.] {
            let mut limiter = Limiter::new(48_000, 0.5);
            let input: Vec<f32> = (0..48_000)
                .map(|i| (i as f32 * hz * std::f32::consts::TAU / 48_000.).sin() * 2.)
                .collect();
            let (mut l, mut r) = (input.clone(), input.clone());
            for (l, r) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
                limiter.process(l, r);
            }
            assert_eq!(limiter.clipped, 0, "{hz} Hz: the gain let samples through");
            assert!(l.iter().all(|s| s.abs() <= 0.5));
        }
    }
}
