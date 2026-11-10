//! A small deterministic generator (SplitMix64): the same seed builds the
//! same harbor on every machine, so the bench's frames repeat.

/// SplitMix64.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator from `seed`.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// A float in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// A float in `[low, high)`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
    /// An index below `n` (`n` > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// A hash of two integers and a seed to `[0, 1)`: value noise's lattice.
pub fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Smooth value noise at `(x, y)`, in `[0, 1)`.
pub fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3. - 2. * t);
    let (u, v) = (s(fx), s(fy));
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    top + (bottom - top) * v
}

/// `octaves` of [`noise2`], each twice as fine and half as strong, in
/// `[0, 1)`.
pub fn fbm2(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut weight, mut total, mut scale) = (0., 1., 0., 1.);
    for octave in 0..octaves {
        sum += noise2(x * scale, y * scale, seed.wrapping_add(octave)) * weight;
        total += weight;
        weight *= 0.5;
        scale *= 2.;
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_the_same_numbers() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::new(7);
        assert!((0..1000).all(|_| (0. ..1.).contains(&c.unit())));
    }

    #[test]
    fn noise_stays_in_its_range() {
        for i in 0..500 {
            let v = fbm2(i as f32 * 0.37, i as f32 * 0.11, 4, 3);
            assert!((0. ..1.).contains(&v), "{v}");
        }
    }
}
