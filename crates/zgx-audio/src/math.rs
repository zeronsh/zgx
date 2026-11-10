//! Exponentials, logarithms and angles made of IEEE 754 addition,
//! multiplication and division alone, so their bits are the same on every
//! platform. The standard library's `exp`, `ln`, `powf`, `sin` and the
//! like call the platform's maths library, whose last bits differ from one
//! system to another (glibc's are not Apple's): a coefficient a bit off
//! compounds, frame after frame, into a difference a reference rendered on
//! another machine can hear. Everything the mixer and its effects compute
//! goes through these instead; a test (`tests/platform_exact.rs`) keeps
//! the rest of the crate from calling the platform's.
//!
//! Each works in double precision to within a few parts in 10^16, so a
//! single-precision result rounds as a correctly rounded library's would.
//! The arguments are reduced the way fdlibm reduces them, and the rest is
//! a Taylor series long enough for double precision.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_6, LN_10};

/// ln 2, in two parts: the first's 32 bits times any exponent are exact.
const LN2_HI: f64 = 0.6931471803691238;
const LN2_LO: f64 = 1.9082149292705877e-10;

/// e^x.
pub(crate) fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.78 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.;
    }
    // x = k ln 2 + r, |r| <= ln 2 / 2.
    let k = (x * std::f64::consts::LOG2_E).round();
    let r = (x - k * LN2_HI) - k * LN2_LO;
    const C: [f64; 13] = [
        0.5,
        0.166_666_666_666_666_66,
        0.041_666_666_666_666_664,
        0.008_333_333_333_333_333,
        0.001_388_888_888_888_889,
        0.000_198_412_698_412_698_4,
        2.480_158_730_158_73e-5,
        2.755_731_922_398_589_3e-6,
        2.755_731_922_398_589e-7,
        2.505_210_838_544_172e-8,
        2.087_675_698_786_81e-9,
        1.605_904_383_682_161_3e-10,
        1.147_074_559_772_972_5e-11,
    ];
    let mut p = C[12];
    for &c in C[..12].iter().rev() {
        p = c + r * p;
    }
    let e = 1. + (r + r * r * p);
    // 2^k in two factors, each a normal number for any k it can be here.
    let k = k as i64;
    let half = k / 2;
    e * power_of_two(half) * power_of_two(k - half)
}

/// 2^n, for n from -1022 to 1023.
fn power_of_two(n: i64) -> f64 {
    f64::from_bits(((n + 1023) as u64) << 52)
}

/// The natural logarithm of `x`: NaN below zero, minus infinity at it.
pub(crate) fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0. {
        return f64::NAN;
    }
    if x == 0. {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    // x = m 2^e, m within a factor of the square root of 2 of 1.
    let (x, mut e) = if x < f64::MIN_POSITIVE {
        (x * power_of_two(54), -54)
    } else {
        (x, 0)
    };
    let bits = x.to_bits();
    e += ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | (1023 << 52));
    if m > std::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    // ln m = 2 atanh(s), s = (m - 1) / (m + 1): 2 (s + s^3 / 3 + ...).
    let f = m - 1.;
    let s = f / (2. + f);
    let z = s * s;
    const C: [f64; 11] = [
        0.333_333_333_333_333_3,
        0.2,
        0.142_857_142_857_142_85,
        0.111_111_111_111_111_1,
        0.090_909_090_909_090_91,
        0.076_923_076_923_076_93,
        0.066_666_666_666_666_67,
        0.058_823_529_411_764_705,
        0.052_631_578_947_368_42,
        0.047_619_047_619_047_616,
        0.043_478_260_869_565_216,
    ];
    let mut p = C[10];
    for &c in C[..10].iter().rev() {
        p = c + z * p;
    }
    let e = e as f64;
    e * LN2_HI + (e * LN2_LO + 2. * (s + s * z * p))
}

/// π/2 in three parts, the first two of 33 bits: any whole number of
/// quarter turns below 2^20 times them is exact.
const PIO2_1: f64 = 1.5707963267341256;
const PIO2_2: f64 = 6.077100506303966e-11;
const PIO2_3: f64 = 2.0222662487959506e-21;

/// The sine and cosine of `x` radians (exact to double precision for
/// |x| below about a million; past that, still the same everywhere).
pub(crate) fn sin_cos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    // x = n π/2 + r, |r| <= π/4.
    let n = (x * std::f64::consts::FRAC_2_PI).round();
    let r = ((x - n * PIO2_1) - n * PIO2_2) - n * PIO2_3;
    let z = r * r;
    const S: [f64; 9] = [
        -0.166_666_666_666_666_66,
        0.008_333_333_333_333_333,
        -0.000_198_412_698_412_698_4,
        2.755_731_922_398_589_3e-6,
        -2.505_210_838_544_172e-8,
        1.605_904_383_682_161_3e-10,
        -7.647_163_731_819_816e-13,
        2.811_457_254_345_520_6e-15,
        -8.220_635_246_624_33e-18,
    ];
    const C: [f64; 9] = [
        -0.5,
        0.041_666_666_666_666_664,
        -0.001_388_888_888_888_889,
        2.480_158_730_158_73e-5,
        -2.755_731_922_398_589e-7,
        2.087_675_698_786_81e-9,
        -1.147_074_559_772_972_5e-11,
        4.779_477_332_387_385e-14,
        -1.561_920_696_858_622_5e-16,
    ];
    let (mut s, mut c) = (S[8], C[8]);
    for (&a, &b) in S[..8].iter().zip(&C[..8]).rev() {
        s = a + z * s;
        c = b + z * c;
    }
    let (s, c) = (r + r * z * s, 1. + z * c);
    match (n as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// The arctangent of `x`, in radians.
pub(crate) fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let (negative, mut t) = (x < 0., x.abs());
    // Past 1, from π/2; past tan(π/12), from π/6: what's left is within
    // tan(π/12) of 0, where the series is short.
    let inverted = t > 1.;
    if inverted {
        t = 1. / t;
    }
    const TAN_PI_12: f64 = 0.267_949_192_431_122_7;
    const SQRT_3: f64 = 1.732_050_807_568_877_2;
    let shifted = t > TAN_PI_12;
    if shifted {
        t = (t * SQRT_3 - 1.) / (t + SQRT_3);
    }
    let z = t * t;
    const C: [f64; 14] = [
        -0.333_333_333_333_333_3,
        0.2,
        -0.142_857_142_857_142_85,
        0.111_111_111_111_111_1,
        -0.090_909_090_909_090_91,
        0.076_923_076_923_076_93,
        -0.066_666_666_666_666_67,
        0.058_823_529_411_764_705,
        -0.052_631_578_947_368_42,
        0.047_619_047_619_047_616,
        -0.043_478_260_869_565_216,
        0.04,
        -0.037_037_037_037_037_035,
        0.034_482_758_620_689_655,
    ];
    let mut p = C[13];
    for &c in C[..13].iter().rev() {
        p = c + z * p;
    }
    let mut a = t + t * z * p;
    if shifted {
        a += FRAC_PI_6;
    }
    if inverted {
        a = FRAC_PI_2 - a;
    }
    if negative { -a } else { a }
}

/// The arccosine of `x` (from -1 to 1), in radians.
pub(crate) fn acos(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1. {
        return f64::NAN;
    }
    if x == -1. {
        return std::f64::consts::PI;
    }
    // cos 2θ = (1 - tan²θ) / (1 + tan²θ).
    2. * atan(((1. - x) / (1. + x)).sqrt())
}

/// e^x, in single precision.
#[inline]
pub(crate) fn expf(x: f32) -> f32 {
    exp(f64::from(x)) as f32
}

/// ln x, in single precision.
#[inline]
pub(crate) fn lnf(x: f32) -> f32 {
    ln(f64::from(x)) as f32
}

/// 10^x, in single precision.
#[inline]
pub(crate) fn exp10f(x: f32) -> f32 {
    exp(f64::from(x) * LN_10) as f32
}

/// log10 x, in single precision.
#[inline]
pub(crate) fn log10f(x: f32) -> f32 {
    (ln(f64::from(x)) / LN_10) as f32
}

/// `a` (zero or more) to the power `b`, in single precision.
pub(crate) fn powf(a: f32, b: f32) -> f32 {
    if b == 0. {
        return 1.;
    }
    if a == 0. {
        return if b > 0. { 0. } else { f32::INFINITY };
    }
    exp(f64::from(b) * ln(f64::from(a))) as f32
}

/// The sine and cosine of `x` radians, in single precision.
#[inline]
pub(crate) fn sin_cosf(x: f32) -> (f32, f32) {
    let (s, c) = sin_cos(f64::from(x));
    (s as f32, c as f32)
}

/// The arccosine of `x`, in single precision.
#[inline]
pub(crate) fn acosf(x: f32) -> f32 {
    acos(f64::from(x)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many units in the last place `got` is from `want`.
    fn ulps(got: f64, want: f64) -> f64 {
        if got == want {
            return 0.;
        }
        (got - want).abs() / (want.abs().max(f64::MIN_POSITIVE) * f64::EPSILON)
    }

    #[test]
    fn each_is_within_a_few_units_in_the_last_place_of_the_library_s() {
        // The library's double precision is the reference here, on
        // whatever machine runs it: within a few units in the last place,
        // and so, rounded to single precision, the correctly rounded float.
        // (Apple's single-precision `expf` is not, for 0.6% of these.)
        let mut worst = [0f64; 5];
        let mut single_differs = 0;
        for i in 0..200_000 {
            let u = f64::from(i) / 200_000.;
            let x = -60. + 120. * u;
            worst[0] = worst[0].max(ulps(exp(x), x.exp()));
            let y = (x * 0.4).exp();
            worst[1] = worst[1].max((ln(y) - y.ln()).abs() / (f64::EPSILON * y.ln().abs().max(1.)));
            let a = -40. + 80. * u;
            let (s, c) = sin_cos(a);
            worst[2] = worst[2].max((s - a.sin()).abs().max((c - a.cos()).abs()) / f64::EPSILON);
            let t = -50. + 100. * u * u;
            worst[3] = worst[3].max(ulps(atan(t), t.atan()));
            let v = -1. + 2. * u;
            worst[4] = worst[4].max((acos(v) - v.acos()).abs() / f64::EPSILON);
            let xf = (x * 0.25) as f32;
            let rounded = f64::from(xf).exp() as f32;
            if expf(xf) != rounded {
                single_differs += 1;
            }
        }
        assert!(worst[0] < 4., "exp: {} ulps", worst[0]);
        assert!(worst[1] < 4., "ln: {} ulps", worst[1]);
        assert!(worst[2] < 4., "sin and cos: {} ulps of 1", worst[2]);
        assert!(worst[3] < 4., "atan: {} ulps", worst[3]);
        assert!(worst[4] < 8., "acos: {} ulps of 1", worst[4]);
        assert!(
            single_differs < 4,
            "expf rounds otherwise {single_differs} times"
        );
    }

    #[test]
    fn the_edges_are_the_library_s() {
        assert_eq!(exp(0.), 1.);
        assert_eq!(exp(1000.), f64::INFINITY);
        assert_eq!(exp(-1000.), 0.);
        assert!(exp(-740.) > 0. && exp(-740.) < 1e-300);
        assert!(exp(f64::NAN).is_nan());
        assert_eq!(ln(1.), 0.);
        assert_eq!(ln(0.), f64::NEG_INFINITY);
        assert!(ln(-1.).is_nan());
        assert!((ln(5e-324) - 5e-324f64.ln()).abs() < 1e-12);
        assert_eq!(sin_cos(0.), (0., 1.));
        assert!(sin_cos(f64::INFINITY).0.is_nan());
        assert_eq!(atan(0.), 0.);
        assert_eq!(acos(1.), 0.);
        assert_eq!(acos(-1.), std::f64::consts::PI);
        assert_eq!(powf(10., 0.), 1.);
        assert_eq!(powf(0., 2.), 0.);
        assert_eq!(powf(0., -2.), f32::INFINITY);
        assert_eq!(expf(0.), 1.);
    }
}
