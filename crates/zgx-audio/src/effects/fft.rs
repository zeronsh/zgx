//! A complex fast Fourier transform of a power-of-two size, radix 2, its
//! tables made once: what the binaural convolution runs on.

/// A complex number: real, imaginary.
pub(crate) type Complex = [f32; 2];

/// The transform of one size: its twiddle factors and bit reversal.
#[derive(Clone, Debug)]
pub(crate) struct Fft {
    size: usize,
    /// Each stage's twiddle factors after the first two, end to end:
    /// e^(-2 pi i k / (2 half)) for k below half, half = 4, 8, ...; and
    /// the same conjugated, for the inverse.
    forward: Box<[Complex]>,
    inverse: Box<[Complex]>,
    reversed: Box<[u32]>,
}

impl Fft {
    /// For `size` points, a power of two of 4 or more. Allocates: off the
    /// audio thread.
    pub fn new(size: usize) -> Fft {
        assert!(size.is_power_of_two() && size >= 4);
        let bits = size.trailing_zeros();
        let mut forward = Vec::with_capacity(size);
        let mut half = 4;
        while half < size {
            for k in 0..half {
                let angle = -std::f64::consts::PI * k as f64 / half as f64;
                let (sin, cos) = crate::math::sin_cos(angle);
                forward.push([cos as f32, sin as f32]);
            }
            half *= 2;
        }
        let inverse = forward.iter().map(|&[r, i]| [r, -i]).collect();
        Fft {
            size,
            forward: forward.into(),
            inverse,
            reversed: (0..size as u32)
                .map(|i| i.reverse_bits() >> (32 - bits))
                .collect(),
        }
    }

    /// Its number of points.
    pub fn size(&self) -> usize {
        self.size
    }

    /// `data` transformed in place: forward, or `inverse` (unscaled: the
    /// caller divides by the size).
    pub fn run(&self, data: &mut [Complex], inverse: bool) {
        let n = self.size;
        let data = &mut data[..n];
        for (i, &j) in self.reversed.iter().enumerate() {
            let j = j as usize;
            if i < j {
                data.swap(i, j);
            }
        }
        // The first two stages together, their twiddles 1 and -i (or i):
        // a radix-4 butterfly on each four.
        let sign = if inverse { -1. } else { 1. };
        for q in data.as_chunks_mut::<4>().0 {
            let [a, b, c, d] = *q;
            let (s, t) = ([a[0] + b[0], a[1] + b[1]], [a[0] - b[0], a[1] - b[1]]);
            let (u, v) = ([c[0] + d[0], c[1] + d[1]], [c[0] - d[0], c[1] - d[1]]);
            // v times -i, forward: (v1, -v0).
            let w = [v[1] * sign, -v[0] * sign];
            *q = [
                [s[0] + u[0], s[1] + u[1]],
                [t[0] + w[0], t[1] + w[1]],
                [s[0] - u[0], s[1] - u[1]],
                [t[0] - w[0], t[1] - w[1]],
            ];
        }
        let twiddles = if inverse {
            &self.inverse
        } else {
            &self.forward
        };
        let (mut half, mut from) = (4, 0);
        while half < n {
            let w = &twiddles[from..from + half];
            for group in data.chunks_exact_mut(half * 2) {
                let (low, high) = group.split_at_mut(half);
                butterflies(low, high, w);
            }
            from += half;
            half *= 2;
        }
    }
}

/// One stage's butterflies on a group: `low` and `high` become their sum
/// and difference, `high` turned by `w` first.
#[inline(always)]
fn butterflies(low: &mut [Complex], high: &mut [Complex], w: &[Complex]) {
    for ((a, b), w) in low.iter_mut().zip(high.iter_mut()).zip(w) {
        let t = [b[0] * w[0] - b[1] * w[1], b[0] * w[1] + b[1] * w[0]];
        *b = [a[0] - t[0], a[1] - t[1]];
        *a = [a[0] + t[0], a[1] + t[1]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transform_is_the_discrete_fourier_transform_and_comes_back() {
        for n in [4, 8, 16, 64, 256] {
            let fft = Fft::new(n);
            let signal: Vec<Complex> = (0..n)
                .map(|i| [((i * 7) % 11) as f32 - 5., ((i * 3) % 5) as f32 - 2.])
                .collect();
            let mut fast = signal.clone();
            fft.run(&mut fast, false);
            for (k, got) in fast.iter().enumerate() {
                let mut want = [0f64; 2];
                for (i, x) in signal.iter().enumerate() {
                    let a = -2. * std::f64::consts::PI * (i * k) as f64 / n as f64;
                    want[0] += f64::from(x[0]) * a.cos() - f64::from(x[1]) * a.sin();
                    want[1] += f64::from(x[0]) * a.sin() + f64::from(x[1]) * a.cos();
                }
                assert!((f64::from(got[0]) - want[0]).abs() < 1e-3, "bin {k}");
                assert!((f64::from(got[1]) - want[1]).abs() < 1e-3, "bin {k}");
            }
            fft.run(&mut fast, true);
            for (x, y) in signal.iter().zip(&fast) {
                assert!(
                    (x[0] - y[0] / n as f32).abs() < 1e-4 && (x[1] - y[1] / n as f32).abs() < 1e-4
                );
            }
        }
    }
}
