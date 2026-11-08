//! Motion blur's mix: a frame's samples summed a byte at a time, and the sum
//! taken back to bytes, rounded: `(sum + n / 2) / n` for `n` samples, exactly.
//!
//! Sums are 16 bits up to 256 samples (256 × 255 and a half of 256 fit),
//! so a pass moves half the memory and a vector holds twice the bytes; more
//! samples sum in 32. The first sample is written, not added to zeros. The
//! average shifts for a power of two of samples, multiplies by a reciprocal
//! and shifts for any other count up to 256, and looks up a table made once
//! past that: no division a byte.

/// The sums of a frame's samples so far, kept from frame to frame.
#[derive(Default)]
pub(crate) struct Blend {
    narrow: Vec<u16>,
    wide: Vec<u32>,
    /// `table[sum]` is the rounded average of `table_samples` samples.
    table: Vec<u8>,
    table_samples: u32,
}

/// Sums fit 16 bits up to this many samples, the rounding half included.
const NARROW: u32 = 256;

impl Blend {
    /// Adds `sample`, the first of `samples` if `first`.
    pub fn add(&mut self, sample: &[u8], first: bool, samples: u32) {
        if samples <= NARROW {
            add(&mut self.narrow, sample, first);
        } else {
            add(&mut self.wide, sample, first);
        }
    }

    /// The rounded average of the `samples` added, into `out`.
    pub fn average_into(&mut self, samples: u32, out: &mut Vec<u8>) {
        let samples = samples.max(1);
        out.clear();
        if samples.is_power_of_two() {
            let shift = samples.trailing_zeros();
            if samples <= NARROW {
                let half = (samples / 2) as u16;
                out.extend(self.narrow.iter().map(|sum| ((sum + half) >> shift) as u8));
            } else {
                let half = samples / 2;
                out.extend(self.wide.iter().map(|sum| ((sum + half) >> shift) as u8));
            }
            return;
        }
        if samples <= NARROW {
            // ⌊x / n⌋ as (x · m) >> 24 with m = ⌊2²⁴ / n⌋ + 1: exact for every
            // x below 2¹⁶, as x · (m · n − 2²⁴) < 2¹⁶ · n ≤ 2²⁴ keeps the
            // excess under 1 / n.
            let m = (1u64 << 24) / u64::from(samples) + 1;
            let half = u64::from(samples / 2);
            out.extend(
                self.narrow
                    .iter()
                    .map(|sum| (((u64::from(*sum) + half) * m) >> 24) as u8),
            );
            return;
        }
        if self.table_samples != samples {
            self.table = (0..=255 * samples)
                .map(|sum| ((sum + samples / 2) / samples) as u8)
                .collect();
            self.table_samples = samples;
        }
        let table = &self.table;
        out.extend(self.wide.iter().map(|sum| table[*sum as usize]));
    }
}

fn add<T: Copy + From<u8> + std::ops::AddAssign>(sums: &mut Vec<T>, sample: &[u8], first: bool) {
    if first {
        sums.clear();
        sums.extend(sample.iter().map(|value| T::from(*value)));
    } else {
        for (sum, value) in sums.iter_mut().zip(sample) {
            *sum += T::from(*value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the mix was before: 32-bit sums, a division a byte.
    fn reference(samples: &[Vec<u8>]) -> Vec<u8> {
        let n = samples.len() as u32;
        let mut sum = vec![0u32; samples[0].len()];
        for sample in samples {
            for (sum, value) in sum.iter_mut().zip(sample) {
                *sum += u32::from(*value);
            }
        }
        sum.iter().map(|sum| ((sum + n / 2) / n) as u8).collect()
    }

    #[test]
    fn the_average_is_exact_for_every_count_and_every_sum() {
        // Each count of samples up to 256, power of two or not (shift and
        // reciprocal), at every sum it can reach (0 to 255 n): about 8 M
        // cases, against a division.
        let mut blend = Blend::default();
        let mut out = Vec::new();
        for n in 1..=NARROW {
            blend.narrow = (0..=255 * n).map(|sum| sum as u16).collect();
            blend.average_into(n, &mut out);
            for (sum, average) in out.iter().enumerate() {
                let exact = (sum as u32 + n / 2) / n;
                assert_eq!(u32::from(*average), exact, "{sum} / {n}");
            }
        }
    }

    #[test]
    fn every_count_of_samples_averages_as_dividing_each_sum_does() {
        // Every byte value at each place, and the extremes, over samples
        // that differ: rounding at each half shows.
        let len = 1024;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        };
        let mut blend = Blend::default();
        let mut out = vec![9; 3];
        for n in [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 255, 256, 257, 300, 512,
        ] {
            let samples: Vec<Vec<u8>> = (0..n)
                .map(|k| {
                    (0..len)
                        .map(|i| match i % 4 {
                            0 => 255,
                            1 => 0,
                            2 => (i / 4 + k) as u8,
                            _ => next(),
                        })
                        .collect()
                })
                .collect();
            for (k, sample) in samples.iter().enumerate() {
                blend.add(sample, k == 0, n);
            }
            blend.average_into(n, &mut out);
            assert_eq!(out, reference(&samples), "{n} samples");
        }
    }
}
