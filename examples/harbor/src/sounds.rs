//! The harbor's sounds, synthesised from a seed: nothing is recorded or
//! loaded. Each is mono, 48 kHz, and loops without a seam (its noise is
//! crossfaded end into start, its tones whole cycles long).
use crate::rng::Rng;

/// Samples a second.
pub const RATE: u32 = 48_000;

/// `seconds` of `f(t, noise)` at [`RATE`], `noise` white noise from
/// `seed`, the last `fade` seconds crossfaded into the first so it loops.
fn synth(seconds: f32, fade: f32, seed: u64, f: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let n = (seconds * RATE as f32) as usize;
    let m = ((fade * RATE as f32) as usize).min(n / 2);
    let mut rng = Rng::new(seed);
    let raw: Vec<f32> = (0..n + m)
        .map(|i| f(i as f32 / RATE as f32, rng.range(-1., 1.)))
        .collect();
    let mut out = raw[..n].to_vec();
    // The tail past `n` fades in over the start, so the end runs into it.
    for i in 0..m {
        let w = i as f32 / m as f32;
        out[i] = raw[n + i] * (1. - w) + raw[i] * w;
    }
    out
}

/// A one-pole low-pass at `hz` over `samples`, in place, as a loop: a
/// first pass round only settles the filter, so the second, which writes,
/// starts from where its end leaves it.
fn low_pass(samples: &mut [f32], hz: f32) {
    let a = 1. - (-std::f32::consts::TAU * hz / RATE as f32).exp();
    let mut y = 0.;
    for &s in samples.iter() {
        y += a * (s - y);
    }
    for s in samples.iter_mut() {
        y += a * (*s - y);
        *s = y;
    }
}

/// A crane's hoist and travel drive: a geared motor's hum (47 Hz and its
/// harmonics, beating slowly) over filtered rumble. Two seconds.
pub fn crane_motor(seed: u64) -> Vec<f32> {
    let mut rumble = synth(2., 0.25, seed, |_, n| n);
    low_pass(&mut rumble, 180.);
    let hum = synth(2., 0., 0, |t, _| {
        let tau = std::f32::consts::TAU;
        let beat = 0.8 + 0.2 * (tau * 0.5 * t).sin();
        beat * (0.5 * (tau * 47. * t).sin()
            + 0.3 * (tau * 94. * t).sin()
            + 0.15 * (tau * 141. * t).sin()
            + 0.08 * (tau * 376. * t).sin())
    });
    hum.iter()
        .zip(&rumble)
        .map(|(h, r)| 0.35 * h + 0.9 * r)
        .collect()
}

/// A ship's diesel generator: a deep firing pulse (25 Hz) and its electric
/// hum (50 Hz, 100 Hz). Two seconds.
pub fn generator(seed: u64) -> Vec<f32> {
    let mut grit = synth(2., 0.25, seed, |_, n| n);
    low_pass(&mut grit, 400.);
    let tone = synth(2., 0., 0, |t, _| {
        let tau = std::f32::consts::TAU;
        let pulse = (tau * 25. * t).sin().max(0.).powi(3);
        0.5 * pulse + 0.25 * (tau * 50. * t).sin() + 0.12 * (tau * 100. * t).sin()
    });
    tone.iter()
        .zip(&grit)
        .zip(synth(2., 0., 0, |t, _| {
            (std::f32::consts::TAU * 25. * t).sin().max(0.)
        }))
        .map(|((t, g), env)| 0.5 * t + 0.4 * g * env)
        .collect()
}

/// A buoy's bell: struck once, its partials (inharmonic, as a bell's)
/// ringing down over six seconds, which is how often it rings.
pub fn bell() -> Vec<f32> {
    synth(6., 0., 0, |t, _| {
        let tau = std::f32::consts::TAU;
        let f = 392.;
        let partials = [
            (0.5, 0.35, 1.2),
            (1., 1., 2.2),
            (1.19, 0.5, 1.8),
            (1.56, 0.4, 1.4),
            (2.0, 0.3, 1.0),
            (2.74, 0.2, 0.6),
        ];
        let strike = (t / 0.004).min(1.);
        strike
            * partials
                .iter()
                .map(|&(ratio, amp, decay)| amp * (-t / decay).exp() * (tau * f * ratio * t).sin())
                .sum::<f32>()
            * 0.3
    })
}

/// Rain: a bed of filtered noise, with drops ticking through it. Four
/// seconds.
pub fn rain(seed: u64) -> Vec<f32> {
    let mut bed = synth(4., 0.5, seed, |_, n| n);
    // Lighter than white: the hiss of many drops far off.
    let mut dull = bed.clone();
    low_pass(&mut dull, 2500.);
    for (b, d) in bed.iter_mut().zip(&dull) {
        *b = 0.25 * (*b - d) + 0.35 * d;
    }
    // Near drops: short decaying ticks at random times.
    let mut rng = Rng::new(seed ^ 0x5eed);
    let len = bed.len();
    for _ in 0..240 {
        let at = rng.below(len);
        let (pitch, gain) = (rng.range(2000., 6000.), rng.range(0.1, 0.4));
        for k in 0..((0.01 * RATE as f32) as usize) {
            let t = k as f32 / RATE as f32;
            let tick = gain * (-t / 0.002).exp() * (std::f32::consts::TAU * pitch * t).sin();
            bed[(at + k) % len] += tick;
        }
    }
    bed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn sounds_are_finite_heard_and_within_full_scale() {
        for (name, s) in [
            ("motor", crane_motor(1)),
            ("generator", generator(2)),
            ("bell", bell()),
            ("rain", rain(3)),
        ] {
            assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 1.), "{name}");
            assert!(rms(&s) > 0.01, "{name} is heard: {}", rms(&s));
        }
    }

    #[test]
    fn loops_have_no_seam() {
        // The step from the last sample to the first is no larger than the
        // largest step inside.
        for s in [crane_motor(1), generator(2), rain(3)] {
            let inside = s.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0., f32::max);
            let seam = (s[0] - s[s.len() - 1]).abs();
            assert!(seam <= inside, "{seam} > {inside}");
        }
    }

    #[test]
    fn a_seed_makes_the_same_sound() {
        assert_eq!(rain(5), rain(5));
        assert_eq!(crane_motor(5), crane_motor(5));
    }
}
