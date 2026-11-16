//! Dynamics: a compressor, whose detector works in decibels (Giannoulis,
//! Massberg and Reiss's design), and a ducker, keyed on another bus.
use crate::{Context, Effect, Format, Parameter};

/// Decibels in a doubling: 20 log10(2).
const DB_PER_OCTAVE: f32 = 6.020_6;

/// The base-2 logarithm of a positive, normal `x`: its exponent, and a
/// polynomial of its mantissa (within 2.2e-6). Arithmetic alone, so the
/// same everywhere, and a fraction of a library's cost.
#[inline(always)]
fn log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    let t = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000) - 1.;
    let p = (((((-0.025_123_194 * t + 0.119_298_21) * t - 0.274_623_23) * t + 0.455_527_07) * t
        - 0.717_557_87)
        * t
        + 1.442_475_3)
        * t
        + 2.123_746_2e-6;
    exponent as f32 + p
}

/// 2 to the power `y` (kept within ±126): its whole part as the exponent,
/// a polynomial for the rest (within 2e-6 of it, and 1 exactly at 0).
#[inline(always)]
fn exp2(y: f32) -> f32 {
    let y = y.clamp(-126., 126.);
    let whole = y.floor();
    let t = y - whole;
    let p = ((((0.001_895_107 * t + 0.008_946_215) * t + 0.055_863_28) * t + 0.240_140_77) * t
        + 0.693_154_6)
        * t
        // 1 exactly: 2^0 is, and a gain of 0 dB must leave a sound as it is.
        + 1.;
    f32::from_bits(((whole as i32 + 127) as u32) << 23) * p
}

/// A one-pole's coefficient for a time constant of `seconds` at `rate`.
fn coefficient(seconds: f32, rate: f32) -> f32 {
    crate::math::expf(-1. / (seconds.max(1e-5) * rate))
}

/// A compressor: above its threshold, every `ratio` decibels in become one
/// out, through a soft knee; the reduction rises over its attack and falls
/// over its release, smoothed in decibels, and the left and right are
/// reduced together (the louder leads).
///
/// Parameters: [`THRESHOLD`](Self::THRESHOLD) in dB (-60 to 0);
/// [`RATIO`](Self::RATIO) (1 to 20); [`ATTACK`](Self::ATTACK) and
/// [`RELEASE`](Self::RELEASE) in seconds (0.0001 to 1, 0.005 to 5);
/// [`KNEE`](Self::KNEE), its width in dB (0 to 24); [`MAKEUP`](Self::MAKEUP),
/// a gain after it in dB (0 to 24).
#[derive(Clone, Debug)]
pub struct Compressor {
    params: [Parameter; 6],
    rate: f32,
    /// The reduction now, in dB (zero or less).
    reduction: f32,
}

impl Compressor {
    pub const THRESHOLD: usize = 0;
    pub const RATIO: usize = 1;
    pub const ATTACK: usize = 2;
    pub const RELEASE: usize = 3;
    pub const KNEE: usize = 4;
    pub const MAKEUP: usize = 5;

    /// `ratio` to 1 above `threshold` dB: 10 ms of attack, 100 ms of
    /// release, a 6 dB knee.
    pub fn new(threshold: f32, ratio: f32) -> Compressor {
        Compressor {
            params: [
                Parameter::new("threshold", threshold.clamp(-60., 0.), -60., 0.),
                Parameter::new("ratio", ratio.clamp(1., 20.), 1., 20.),
                Parameter::new("attack", 0.01, 0.0001, 1.),
                Parameter::new("release", 0.1, 0.005, 5.),
                Parameter::new("knee", 6., 0., 24.),
                Parameter::new("makeup", 0., 0., 24.),
            ],
            rate: 48_000.,
            reduction: 0.,
        }
    }

    /// Its attack and release, in seconds.
    pub fn times(mut self, attack: f32, release: f32) -> Compressor {
        self.params[Self::ATTACK].default = attack.clamp(0.0001, 1.);
        self.params[Self::RELEASE].default = release.clamp(0.005, 5.);
        self
    }

    /// Its knee's width and its makeup gain, in dB.
    pub fn knee(mut self, knee: f32, makeup: f32) -> Compressor {
        self.params[Self::KNEE].default = knee.clamp(0., 24.);
        self.params[Self::MAKEUP].default = makeup.clamp(0., 24.);
        self
    }
}

/// The reduction, in dB, the curve asks of a level of `x` dB.
#[inline(always)]
fn curve(x: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let over = x - threshold;
    let slope = 1. / ratio - 1.;
    if 2. * over <= -knee {
        0.
    } else if 2. * over >= knee {
        slope * over
    } else {
        // Within the knee: a quadratic from no reduction to the slope.
        let into = over + knee / 2.;
        slope * into * into / (2. * knee)
    }
}

impl Effect for Compressor {
    fn parameters(&self) -> &[Parameter] {
        &self.params
    }

    fn prepare(&mut self, format: Format, _max_block: usize) {
        self.rate = format.rate as f32;
        self.reduction = 0.;
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let frames = block.as_chunks_mut::<2>().0;
        let steady = (0..6).all(|p| context.values(p).is_none());
        let mut values = [0, 1, 2, 3, 4, 5].map(|p| context.value(p));
        let mut coefficients = [
            coefficient(values[2], self.rate),
            coefficient(values[3], self.rate),
        ];
        let mut makeup = values[5];
        for (i, frame) in frames.iter_mut().enumerate() {
            if !steady {
                values = [0, 1, 2, 3, 4, 5].map(|p| context.value_at(p, i));
                coefficients = [
                    coefficient(values[2], self.rate),
                    coefficient(values[3], self.rate),
                ];
                makeup = values[5];
            }
            let [threshold, ratio, _, _, knee, _] = values;
            let peak = frame[0].abs().max(frame[1].abs()).max(1e-9);
            let wanted = curve(DB_PER_OCTAVE * log2(peak), threshold, ratio, knee);
            // Deeper is the attack; back up is the release.
            let c = if wanted < self.reduction {
                coefficients[0]
            } else {
                coefficients[1]
            };
            self.reduction = wanted + c * (self.reduction - wanted);
            let gain = exp2((self.reduction + makeup) / DB_PER_OCTAVE);
            *frame = [frame[0] * gain, frame[1] * gain];
        }
    }
}

/// A ducker: lowers its bus by [`DEPTH`](Self::DEPTH) decibels while the
/// bus it's keyed on (its side chain) is louder than
/// [`THRESHOLD`](Self::THRESHOLD), going down over its attack and back
/// over its release: music under dialogue. Add it with
/// [`Bus::duck`](crate::Bus::duck), or
/// [`Bus::add_effect_keyed`](crate::Bus::add_effect_keyed); without a side
/// chain it does nothing.
///
/// Parameters: [`DEPTH`](Self::DEPTH) in dB (-60 to 0);
/// [`THRESHOLD`](Self::THRESHOLD), the side chain's level in dB that ducks
/// it (-80 to 0); [`ATTACK`](Self::ATTACK) and [`RELEASE`](Self::RELEASE)
/// in seconds (0.001 to 5).
#[derive(Clone, Debug)]
pub struct Duck {
    params: [Parameter; 4],
    rate: f32,
    /// Its gain now, in dB.
    gain: f32,
}

impl Duck {
    pub const DEPTH: usize = 0;
    pub const THRESHOLD: usize = 1;
    pub const ATTACK: usize = 2;
    pub const RELEASE: usize = 3;

    /// `depth` dB down (-9: well under) whenever its side chain is above
    /// -40 dB, over `attack` seconds, and back over `release`.
    pub fn new(depth: f32, attack: f32, release: f32) -> Duck {
        Duck {
            params: [
                Parameter::new("depth", depth.clamp(-60., 0.), -60., 0.),
                Parameter::new("threshold", -40., -80., 0.),
                Parameter::new("attack", attack.clamp(0.001, 5.), 0.001, 5.),
                Parameter::new("release", release.clamp(0.001, 5.), 0.001, 5.),
            ],
            rate: 48_000.,
            gain: 0.,
        }
    }

    /// The side chain's level, in dB, above which it ducks.
    pub fn threshold(mut self, db: f32) -> Duck {
        self.params[Self::THRESHOLD].default = db.clamp(-80., 0.);
        self
    }
}

impl Effect for Duck {
    fn parameters(&self) -> &[Parameter] {
        &self.params
    }

    fn prepare(&mut self, format: Format, _max_block: usize) {
        self.rate = format.rate as f32;
        self.gain = 0.;
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let Some(level) = context.side_chain() else {
            return;
        };
        let level = DB_PER_OCTAVE * log2(level.max(1e-9));
        let frames = block.as_chunks_mut::<2>().0;
        // The coefficients once, unless a parameter moves in the block.
        let steady = (0..4).all(|p| context.values(p).is_none());
        let mut values = [0, 1, 2, 3].map(|p| context.value(p));
        let mut coefficients = [
            coefficient(values[2], self.rate),
            coefficient(values[3], self.rate),
        ];
        for (i, frame) in frames.iter_mut().enumerate() {
            if !steady {
                values = [0, 1, 2, 3].map(|p| context.value_at(p, i));
                coefficients = [
                    coefficient(values[2], self.rate),
                    coefficient(values[3], self.rate),
                ];
            }
            let [depth, threshold, _, _] = values;
            let target = if level > threshold { depth } else { 0. };
            let c = if target < self.gain {
                coefficients[0]
            } else {
                coefficients[1]
            };
            self.gain = target + c * (self.gain - target);
            let gain = exp2(self.gain / DB_PER_OCTAVE);
            *frame = [frame[0] * gain, frame[1] * gain];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logarithm_and_power_are_within_a_few_parts_in_a_million() {
        let mut worst = 0f64;
        for i in 0..200_000 {
            let x = 1e-9f64 * 1.000_15f64.powi(i);
            let want = x.log2();
            worst = worst.max((f64::from(log2(x as f32)) - want).abs());
        }
        assert!(worst < 5e-6, "log2 off by {worst:e}");
        let mut worst = 0f64;
        for i in -40_000..=40_000 {
            let y = f64::from(i) * 0.001;
            let want = y.exp2();
            worst = worst.max(((f64::from(exp2(y as f32)) - want) / want).abs());
        }
        assert!(worst < 2e-6, "exp2 off by {worst:e}");
        assert_eq!(exp2(0.), 1.);
    }
}
