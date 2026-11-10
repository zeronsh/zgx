//! Parameters that move to a new value over a number of frames, a step
//! each sample, so a change never zips or clicks.

/// The shape of a change from one value to another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Curve {
    /// In straight steps of value.
    #[default]
    Linear,
    /// Power moves in straight steps, so a fade out under a fade in keeps
    /// the sum as loud: the curve for crossfades.
    EqualPower,
    /// Decibels move in straight steps: the curve that sounds even.
    Exponential,
}

/// How a parameter reaches its new value: over how long, and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub(crate) seconds: f64,
    pub(crate) curve: Curve,
}

impl Ramp {
    /// At once, from the next frame.
    pub const NONE: Ramp = Ramp {
        seconds: 0.,
        curve: Curve::Linear,
    };
    /// Over `ms` milliseconds, in straight steps.
    pub fn ms(ms: f64) -> Ramp {
        Ramp::seconds(ms / 1000.)
    }
    /// Over `seconds`, in straight steps.
    pub fn seconds(seconds: f64) -> Ramp {
        Ramp {
            seconds: seconds.max(0.),
            curve: Curve::Linear,
        }
    }
    /// The same length with another shape.
    pub fn curve(self, curve: Curve) -> Ramp {
        Ramp { curve, ..self }
    }
    pub(crate) fn frames(self, rate: u32) -> u32 {
        (self.seconds * f64::from(rate))
            .round()
            .min(f64::from(u32::MAX)) as u32
    }
}

/// Zero, as the exponential curve reaches it: -80 dB, then zero on the
/// ramp's last frame.
const FLOOR: f32 = 1e-4;

/// A value per sample on its way to a target.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Smoothed {
    from: f32,
    to: f32,
    /// The value at the next sample.
    value: f32,
    /// Samples since the ramp began.
    done: u32,
    length: u32,
    /// `1 / length`.
    step: f32,
    curve: Curve,
}

impl Smoothed {
    pub fn new(value: f32) -> Self {
        Smoothed {
            from: value,
            to: value,
            value,
            done: 0,
            length: 0,
            step: 0.,
            curve: Curve::Linear,
        }
    }

    /// Moves from wherever it is now to `to` over `frames` samples.
    pub fn set(&mut self, to: f32, frames: u32, curve: Curve) {
        self.from = self.value;
        self.to = to;
        self.done = 0;
        self.length = frames;
        self.curve = curve;
        if frames == 0 {
            self.value = to;
        } else {
            self.step = 1. / frames as f32;
        }
    }

    /// Whether it has reached its target, so every sample is the same.
    #[inline]
    pub fn settled(&self) -> bool {
        self.done >= self.length
    }

    /// The value at the next sample.
    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Where it's going.
    pub fn target(&self) -> f32 {
        self.to
    }

    /// The value at the next sample, then a sample on.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let value = self.value;
        if self.done < self.length {
            self.done += 1;
            self.value = self.at(self.done);
        }
        value
    }

    /// Skips `frames` samples.
    pub fn advance(&mut self, frames: usize) {
        if self.done < self.length {
            self.done = self
                .done
                .saturating_add(frames.min(u32::MAX as usize) as u32)
                .min(self.length);
            self.value = self.at(self.done);
        }
    }

    /// The next `out.len()` samples' values.
    pub fn fill(&mut self, out: &mut [f32]) {
        let ramped = ((self.length - self.done.min(self.length)) as usize).min(out.len());
        let (a, b, step, done) = (self.from, self.to, self.step, self.done);
        // Each sample's value from its index alone, as `at` makes it: no
        // value carried from one to the next, so the loops vectorise.
        let ramp = &mut out[..ramped];
        match self.curve {
            Curve::Linear => {
                for (i, slot) in ramp.iter_mut().enumerate() {
                    let t = (done + i as u32) as f32 * step;
                    *slot = a + (b - a) * t;
                }
            }
            Curve::EqualPower => {
                for (i, slot) in ramp.iter_mut().enumerate() {
                    let t = (done + i as u32) as f32 * step;
                    *slot = (a * a * (1. - t) + b * b * t).sqrt();
                }
            }
            Curve::Exponential => {
                // The logarithms once; each sample as `at` makes it.
                let (la, lb) = logarithms(a, b);
                for (i, slot) in ramp.iter_mut().enumerate() {
                    let done = done + i as u32;
                    *slot = if done == 0 {
                        a
                    } else {
                        exponential(la, lb, done as f32 * step)
                    };
                }
            }
        }
        self.done += ramped as u32;
        self.value = self.at(self.done);
        out[ramped..].fill(self.value);
    }

    fn at(&self, done: u32) -> f32 {
        if done >= self.length {
            return self.to;
        }
        if done == 0 {
            // Where it was: exactly, which `exp(ln(from))` isn't.
            return self.from;
        }
        let t = done as f32 * self.step;
        let (a, b) = (self.from, self.to);
        // The two curves for gains take values as gains: none below zero.
        match self.curve {
            Curve::Linear => a + (b - a) * t,
            Curve::EqualPower => (a * a * (1. - t) + b * b * t).sqrt(),
            Curve::Exponential => {
                let (la, lb) = logarithms(a, b);
                exponential(la, lb, t)
            }
        }
    }
}

/// The logarithms an exponential curve moves between: none below
/// [`FLOOR`], so a fade to silence has somewhere to go.
#[inline(always)]
fn logarithms(a: f32, b: f32) -> (f32, f32) {
    (
        crate::math::lnf(a.max(FLOOR)),
        crate::math::lnf(b.max(FLOOR)),
    )
}

/// An exponential curve's value `t` of the way.
#[inline(always)]
fn exponential(la: f32, lb: f32, t: f32) -> f32 {
    crate::math::expf(la + (lb - la) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linear_ramp_takes_its_length_and_lands_exactly() {
        let mut gain = Smoothed::new(1.);
        gain.set(0., 4, Curve::Linear);
        let values: Vec<f32> = (0..6).map(|_| gain.next()).collect();
        assert_eq!(values, [1., 0.75, 0.5, 0.25, 0., 0.]);
        assert!(gain.settled());
    }

    #[test]
    fn filling_matches_stepping_and_skipping_matches_both() {
        let (mut a, mut b, mut c) = (Smoothed::new(0.2), Smoothed::new(0.2), Smoothed::new(0.2));
        for s in [&mut a, &mut b, &mut c] {
            s.set(0.9, 100, Curve::Exponential);
        }
        let mut filled = [0f32; 70];
        a.fill(&mut filled);
        let stepped: Vec<f32> = (0..70).map(|_| b.next()).collect();
        assert_eq!(filled.as_slice(), stepped.as_slice());
        c.advance(70);
        assert_eq!(c.value(), b.value());
    }

    #[test]
    fn an_equal_power_fade_keeps_the_sum_of_powers() {
        let (mut out, mut into) = (Smoothed::new(1.), Smoothed::new(0.));
        out.set(0., 480, Curve::EqualPower);
        into.set(1., 480, Curve::EqualPower);
        for _ in 0..480 {
            let (a, b) = (out.next(), into.next());
            assert!((a * a + b * b - 1.).abs() < 1e-5);
        }
    }

    #[test]
    fn an_exponential_fade_halves_its_decibels_halfway() {
        let mut gain = Smoothed::new(1.);
        gain.set(0.01, 100, Curve::Exponential);
        gain.advance(50);
        assert!((20. * gain.value().log10() + 20.).abs() < 1e-3);
        gain.set(0., 10, Curve::Exponential);
        gain.advance(10);
        assert_eq!(gain.value(), 0.);
    }
}
