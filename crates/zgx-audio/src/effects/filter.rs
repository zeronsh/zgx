//! Filters: the state-variable filter of Andrew Simper (Cytomic), whose
//! integrators are solved by the trapezoidal rule. Its response is the
//! analog prototype's through the bilinear transform, the same as the
//! audio-cookbook biquads', but it stays well behaved while its cutoff
//! moves a frame at a time and at low cutoffs in single precision.
use crate::{Context, Effect, Format, Parameter};
use std::f64::consts::PI;
use std::time::Duration;

/// How steeply a low-pass or high-pass falls past its cutoff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slope {
    /// 12 dB an octave: one section.
    #[default]
    Db12,
    /// 24 dB an octave: two sections, Butterworth at the default resonance.
    Db24,
}

/// The resonance at which a 12 dB filter is flattest (Butterworth).
pub const FLAT: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// A 4th-order Butterworth's two sections' Qs, over [`FLAT`]: a 24 dB
/// filter's resonance scales both.
const BUTTERWORTH_4: [f32; 2] = [
    0.541_196_1 / std::f32::consts::FRAC_1_SQRT_2,
    1.306_563 / std::f32::consts::FRAC_1_SQRT_2,
];

/// One section's coefficients: `a1`, `a2`, `a3` as Simper names them, and
/// what's mixed out of it: the input, the band and the low.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coefficients {
    a: [f32; 3],
    mix: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Low,
    High,
    Peak,
}

impl Coefficients {
    /// For a cutoff (or centre) `hz` at `rate`, resonance `q`, and for a
    /// peak, `db` of gain at its centre. In single precision, the same
    /// whether the parameters hold still or move a frame at a time, so a
    /// block split anywhere gives the same samples.
    #[inline(always)]
    fn new(shape: Shape, hz: f32, q: f32, db: f32, rate: &Rate) -> Coefficients {
        let g = tan(hz.clamp(rate.lowest, rate.highest) * rate.scale);
        let (k, mix) = match shape {
            Shape::Low => (1. / q, [0., 0., 1.]),
            Shape::High => {
                let k = 1. / q;
                (k, [1., -k, -1.])
            }
            Shape::Peak => {
                let a = crate::math::exp10f(db / 40.);
                let k = 1. / (q * a);
                (k, [1., k * (a * a - 1.), 0.])
            }
        };
        let a1 = 1. / (1. + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Coefficients {
            a: [a1, a2, a3],
            mix,
        }
    }
}

/// What the coefficients need of the rate: the angle a hertz is, and the
/// frequencies a cutoff is kept between (just above 0, just under half the
/// rate).
#[derive(Clone, Copy, Debug)]
struct Rate {
    scale: f32,
    lowest: f32,
    highest: f32,
}

impl Rate {
    fn new(rate: u32) -> Rate {
        let rate = rate.max(1) as f32;
        Rate {
            scale: std::f32::consts::PI / rate,
            lowest: 1e-6 * rate,
            highest: 0.499 * rate,
        }
    }
}

/// The tangent of `x`, from 0 to just under a quarter turn: Cephes's
/// single-precision polynomial (about 1e-7 relative) below an eighth of a
/// turn, its reciprocal at the complement above. Arithmetic alone, so the
/// same everywhere, where a library's `tanf` may differ by a bit between
/// systems; and a fraction of its cost.
#[inline(always)]
fn tan(x: f32) -> f32 {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    let poly = |x: f32| {
        let z = x * x;
        let p = ((((9.385_402e-3 * z + 3.119_922_3e-3) * z + 2.443_013_5e-2) * z + 5.341_128e-2)
            * z
            + 1.333_88e-1)
            * z
            + 3.333_315_7e-1;
        p * z * x + x
    };
    if x <= FRAC_PI_4 {
        poly(x)
    } else {
        1. / poly(FRAC_PI_2 - x)
    }
}

/// One section's state, both channels: the two integrators' charges.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl State {
    /// One frame through the section, its output mixed for `shape`.
    #[inline(always)]
    fn step(&mut self, c: &Coefficients, shape: Shape, input: [f32; 2]) -> [f32; 2] {
        let [a1, a2, a3] = c.a;
        let [m0, m1, m2] = c.mix;
        let mut out = [0.; 2];
        for ch in 0..2 {
            let x = input[ch];
            let v3 = x - self.ic2[ch];
            let v1 = a1 * self.ic1[ch] + a2 * v3;
            let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
            self.ic1[ch] = 2. * v1 - self.ic1[ch];
            self.ic2[ch] = 2. * v2 - self.ic2[ch];
            out[ch] = match shape {
                Shape::Low => v2,
                Shape::High => m0 * x + m1 * v1 + m2 * v2,
                Shape::Peak => m0 * x + m1 * v1,
            };
        }
        out
    }
}

/// Every frame of `frames` through `sections` sections of `shape`, the
/// coefficients holding still: a loop of its own for each shape, so each
/// mixes only what its shape needs.
#[inline(always)]
fn run<const SECTIONS: usize>(
    shape: Shape,
    states: &mut [State; 2],
    coefficients: &[Coefficients; 2],
    frames: &mut [[f32; 2]],
) {
    let [s0, s1] = states;
    let [c0, c1] = coefficients;
    for frame in frames.iter_mut() {
        let mut x = s0.step(c0, shape, *frame);
        if SECTIONS == 2 {
            x = s1.step(c1, shape, x);
        }
        *frame = x;
    }
}

/// The filter all three effects are: up to two sections, the parameters
/// they were last made for, and the rate.
#[derive(Clone, Debug)]
struct Svf {
    shape: Shape,
    sections: usize,
    states: [State; 2],
    coefficients: [Coefficients; 2],
    /// The parameters the coefficients were made for (NaN: none yet).
    made: [f32; 3],
    rate: Rate,
    params: [Parameter; 3],
}

impl Svf {
    fn new(shape: Shape, sections: usize, params: [Parameter; 3]) -> Svf {
        Svf {
            shape,
            sections,
            states: [State::default(); 2],
            coefficients: [Coefficients::default(); 2],
            made: [f32::NAN; 3],
            rate: Rate::new(48_000),
            params,
        }
    }

    /// The coefficients for these parameters: frequency, resonance, gain.
    #[inline(always)]
    fn make<const SECTIONS: usize>(&mut self, shape: Shape, values: [f32; 3]) {
        if values == self.made {
            return;
        }
        self.made = values;
        let [hz, q, db] = values;
        for (c, scale) in self.coefficients[..SECTIONS].iter_mut().zip(BUTTERWORTH_4) {
            let q = if SECTIONS == 2 { q * scale } else { q };
            *c = Coefficients::new(shape, hz, q, db, &self.rate);
        }
    }

    /// Every frame of `frames` through `SECTIONS` sections of `shape`, the
    /// coefficients made again on each frame a parameter moves: a loop of
    /// its own for each shape and count, as when they hold still.
    #[inline(always)]
    fn sweep<const SECTIONS: usize>(
        &mut self,
        shape: Shape,
        frames: &mut [[f32; 2]],
        context: &Context<'_>,
    ) {
        let still = [0, 1, 2].map(|p| context.value(p));
        let moving = [0, 1, 2].map(|p| context.values(p));
        for (i, frame) in frames.iter_mut().enumerate() {
            let values = [0, 1, 2].map(|p| moving[p].map_or(still[p], |v| v[i]));
            self.make::<SECTIONS>(shape, values);
            let mut x = self.states[0].step(&self.coefficients[0], shape, *frame);
            if SECTIONS == 2 {
                x = self.states[1].step(&self.coefficients[1], shape, x);
            }
            *frame = x;
        }
    }

    fn prepare(&mut self, format: Format) {
        self.rate = Rate::new(format.rate);
        self.states = [State::default(); 2];
        self.made = [f32::NAN; 3];
    }

    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let moving = (0..3).any(|p| context.values(p).is_some());
        let frames = block.as_chunks_mut::<2>().0;
        if !moving {
            let values = [0, 1, 2].map(|p| context.value(p));
            let shape = self.shape;
            if self.sections == 2 {
                self.make::<2>(shape, values);
            } else {
                self.make::<1>(shape, values);
            }
            let (states, c) = (&mut self.states, &self.coefficients);
            match (shape, self.sections) {
                (Shape::Low, 1) => run::<1>(Shape::Low, states, c, frames),
                (Shape::Low, _) => run::<2>(Shape::Low, states, c, frames),
                (Shape::High, 1) => run::<1>(Shape::High, states, c, frames),
                (Shape::High, _) => run::<2>(Shape::High, states, c, frames),
                (Shape::Peak, _) => run::<1>(Shape::Peak, states, c, frames),
            }
        } else {
            match (self.shape, self.sections) {
                (Shape::Low, 1) => self.sweep::<1>(Shape::Low, frames, context),
                (Shape::Low, _) => self.sweep::<2>(Shape::Low, frames, context),
                (Shape::High, 1) => self.sweep::<1>(Shape::High, frames, context),
                (Shape::High, _) => self.sweep::<2>(Shape::High, frames, context),
                (Shape::Peak, _) => self.sweep::<1>(Shape::Peak, frames, context),
            }
        }
    }

    /// Until its ringing has fallen 90 dB: a section's envelope decays as
    /// e^(-pi f t / Q), so that's ln(10^4.5) Q / (pi f).
    fn tail(&self) -> Duration {
        let [hz, q, db] = self.made.map(f64::from);
        if hz.is_nan() {
            return Duration::ZERO;
        }
        let q = match self.shape {
            Shape::Peak => q * crate::math::exp(db / 40. * std::f64::consts::LN_10),
            _ if self.sections == 2 => q * f64::from(BUTTERWORTH_4[1]),
            _ => q,
        };
        Duration::from_secs_f64(10.36 * q / (PI * hz.max(1.)))
    }
}

macro_rules! pass {
    ($name:ident, $shape:expr, $what:literal, $default:literal) => {
        #[doc = concat!("A ", $what, " filter, state-variable, 12 or 24 dB an octave.")]
        ///
        /// Parameters: [`CUTOFF`](Self::CUTOFF) in hertz (10 to 24 000),
        /// [`RESONANCE`](Self::RESONANCE) as a Q (0.1 to 20; [`FLAT`], the
        /// default, has no peak).
        #[derive(Clone, Debug)]
        pub struct $name(Svf);

        impl $name {
            pub const CUTOFF: usize = 0;
            pub const RESONANCE: usize = 1;

            /// Cutting off at `cutoff` hertz, flat below it, 12 dB an octave.
            pub fn new(cutoff: f32) -> $name {
                $name(Svf::new(
                    $shape,
                    1,
                    [
                        Parameter::new("cutoff", cutoff.clamp(10., 24_000.), 10., 24_000.),
                        Parameter::new("resonance", FLAT, 0.1, 20.),
                        // A pass filter has no gain; kept at 0 so it has three.
                        Parameter::new("gain", 0., 0., 0.),
                    ],
                ))
            }

            /// Its resonance, as a Q.
            pub fn resonance(mut self, q: f32) -> $name {
                self.0.params[1].default = q.clamp(0.1, 20.);
                self
            }

            /// How steeply it falls.
            pub fn slope(mut self, slope: Slope) -> $name {
                self.0.sections = match slope {
                    Slope::Db12 => 1,
                    Slope::Db24 => 2,
                };
                self
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::new($default)
            }
        }

        impl Effect for $name {
            fn parameters(&self) -> &[Parameter] {
                &self.0.params
            }
            fn prepare(&mut self, format: Format, _max_block: usize) {
                self.0.prepare(format);
            }
            fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
                self.0.process(block, context);
            }
            fn tail(&self) -> Duration {
                self.0.tail()
            }
        }
    };
}

pass!(LowPass, Shape::Low, "low-pass", 1000.);
pass!(HighPass, Shape::High, "high-pass", 100.);

/// A peaking equaliser band: a bell of gain around a centre frequency,
/// state-variable (the cookbook's peaking EQ, the same response).
///
/// Parameters: [`FREQUENCY`](Self::FREQUENCY) in hertz (10 to 24 000),
/// [`Q`](Self::Q) (0.1 to 20: higher is narrower), [`GAIN`](Self::GAIN) in
/// decibels (-36 to 36).
#[derive(Clone, Debug)]
pub struct Peaking(Svf);

impl Peaking {
    pub const FREQUENCY: usize = 0;
    pub const Q: usize = 1;
    pub const GAIN: usize = 2;

    /// `db` decibels at `frequency` hertz, `q` wide.
    pub fn new(frequency: f32, db: f32, q: f32) -> Peaking {
        Peaking(Svf::new(
            Shape::Peak,
            1,
            [
                Parameter::new("frequency", frequency.clamp(10., 24_000.), 10., 24_000.),
                Parameter::new("q", q.clamp(0.1, 20.), 0.1, 20.),
                Parameter::new("gain", db.clamp(-36., 36.), -36., 36.),
            ],
        ))
    }
}

impl Default for Peaking {
    fn default() -> Peaking {
        Peaking::new(1000., 0., 1.)
    }
}

impl Effect for Peaking {
    fn parameters(&self) -> &[Parameter] {
        &self.0.params
    }
    fn prepare(&mut self, format: Format, _max_block: usize) {
        self.0.prepare(format);
    }
    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        self.0.process(block, context);
    }
    fn tail(&self) -> Duration {
        self.0.tail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The magnitude of the analog prototype prewarped as the bilinear
    /// transform warps it: what the filter's response must be at `hz`.
    fn expected(shape: Shape, cutoff: f64, q: f64, db: f64, hz: f64, rate: f64) -> f64 {
        // The digital frequency maps to an analog one by tan; the prototype
        // is normalised to the prewarped cutoff.
        let w = (PI * hz / rate).tan() / (PI * cutoff / rate).tan();
        let (re, im) = (1. - w * w, w / q);
        let den = (re * re + im * im).sqrt();
        match shape {
            Shape::Low => 1. / den,
            Shape::High => w * w / den,
            Shape::Peak => {
                let a = 10f64.powf(db / 40.);
                // (s² + s A/Q + 1) / (s² + s/(A Q) + 1)
                let num = (re * re + (w * a / q).powi(2)).sqrt();
                let den = (re * re + (w / (a * q)).powi(2)).sqrt();
                num / den
            }
        }
    }

    #[test]
    fn the_tangent_is_within_a_few_parts_in_ten_million_up_to_its_limit() {
        // Up to the largest angle a filter asks for: 0.499 of the rate.
        let mut worst = 0f64;
        for i in 1..=100_000 {
            let x = (i as f64 / 100_000. * 0.499 * PI) as f32;
            let want = f64::from(x).tan();
            worst = worst.max(((f64::from(tan(x)) - want) / want).abs());
        }
        // Near the pole the angle's own rounding (an f32 a few thousandths
        // under a quarter turn) is 4e-5 of what's left of it.
        assert!(worst < 2e-5, "{worst:e}");
        let mut below = 0f64;
        for i in 1..=100_000 {
            let x = (i as f64 / 100_000. * 0.45 * PI) as f32;
            let want = f64::from(x).tan();
            below = below.max(((f64::from(tan(x)) - want) / want).abs());
        }
        assert!(below < 5e-7, "{below:e}");
    }

    #[test]
    fn a_section_s_coefficients_give_the_prototype_s_response() {
        let rate = 48_000.;
        for (shape, cutoff, q, db) in [
            (Shape::Low, 1000., std::f64::consts::FRAC_1_SQRT_2, 0.),
            (Shape::Low, 30., 4., 0.),
            (Shape::High, 5000., 0.5, 0.),
            (Shape::Peak, 2000., 1.4, 9.),
            (Shape::Peak, 300., 0.7, -12.),
        ] {
            let c = Coefficients::new(
                shape,
                cutoff as f32,
                q as f32,
                db as f32,
                &Rate::new(rate as u32),
            );
            for hz in [20., 100., 440., 1000., 3000., 10_000., 20_000.] {
                // Its response at hz, measured: a long sine through it.
                let mut state = State::default();
                let (mut re, mut im) = (0f64, 0f64);
                let n = 96_000;
                for i in 0..n {
                    let phase = 2. * PI * hz * i as f64 / rate;
                    let [out, _] = state.step(&c, shape, [phase.sin() as f32, 0.]);
                    if i >= n / 2 {
                        re += f64::from(out) * phase.sin();
                        im += f64::from(out) * phase.cos();
                    }
                }
                let got = 2. * (re * re + im * im).sqrt() / (n / 2) as f64;
                let want = expected(shape, cutoff, q, db, hz, rate);
                let error_db = 20. * (got / want).log10();
                assert!(
                    error_db.abs() < 0.01,
                    "{shape:?} {cutoff} Hz Q {q} {db} dB, at {hz} Hz: {got} against {want}"
                );
            }
        }
    }
}
