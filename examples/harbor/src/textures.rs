//! Textures generated at start, from noise and a seed: nothing is loaded.
use crate::rng::{Rng, fbm2, noise2};
use zgx_3d::Texture;

/// A texture `width` by `height` of `f(x, y)`'s linear-ish RGB in 0..1,
/// written as sRGB bytes, opaque.
fn paint(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Texture {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = f(x, y);
            for c in [r, g, b] {
                pixels.push((c.clamp(0., 1.) * 255. + 0.5) as u8);
            }
            pixels.push(255);
        }
    }
    Texture::from_rgba8(width, height, pixels).expect("a texture's size")
}

/// A container's paint, worn: near white, to be multiplied by each
/// container's own colour. Darker rails top and bottom, grime rising from
/// the bottom, rust running down from the top rail, an anonymous plate.
/// Laid out as `shapes::container` reads it: U along a side, V down it.
pub fn container_wear(seed: u64) -> Texture {
    let (w, h) = (512u32, 256u32);
    let mut rng = Rng::new(seed);
    // Streaks: where along the side, how far down, how strong.
    let streaks: Vec<(f32, f32, f32, f32)> = (0..26)
        .map(|_| {
            (
                rng.unit(),
                rng.range(0.15, 0.85),
                rng.range(0.15, 0.55),
                rng.range(0.002, 0.008),
            )
        })
        .collect();
    let s = seed as u32;
    paint(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let mut k = 0.92;
        // Rails.
        if !(0.045..=0.95).contains(&v) {
            k *= 0.72;
        }
        // Grime from the bottom, patchy.
        let patch = fbm2(u * 14., v * 6., 4, s);
        k *= 1. - 0.28 * v.powi(3) * (0.6 + patch);
        // Faded patches of paint.
        k *= 0.93 + 0.07 * noise2(u * 40., v * 20., s.wrapping_add(9));
        let mut rgb = [k, k, k];
        for &(at, length, strength, half_width) in &streaks {
            let across = ((u - at) / half_width).abs();
            if across < 1. && v > 0.045 && v < 0.045 + length {
                let fade = (1. - across) * (1. - (v - 0.045) / length) * strength;
                rgb[0] *= 1. - 0.25 * fade;
                rgb[1] *= 1. - 0.55 * fade;
                rgb[2] *= 1. - 0.75 * fade;
            }
        }
        // A plate near the doors' end: a light panel with dark bars.
        if (0.86..0.97).contains(&u) && (0.12..0.3).contains(&v) {
            let bar = ((v - 0.12) * 60.) as i32 % 3 == 0 && u < 0.95;
            rgb = if bar { [0.12; 3] } else { [0.95; 3] };
        }
        rgb
    })
}

/// Concrete slabs `512` px a tile, joints along two edges: the quay's
/// apron, placed with `MapPlacement::scaled` so a tile covers 10 m.
pub fn concrete(seed: u64) -> Texture {
    let n = 512u32;
    let s = seed as u32;
    paint(n, n, |x, y| {
        let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
        let mut k = 0.62 + 0.08 * fbm2(u * 24., v * 24., 5, s) - 0.04;
        k *= 0.96 + 0.04 * noise2(u * 300., v * 300., s.wrapping_add(3));
        // Joints every half tile.
        let joint = |t: f32| ((t * 2.).fract() * n as f32 / 2.) < 2.;
        if joint(u) || joint(v) {
            k *= 0.55;
        }
        // Tyre marks, faint.
        let tyre = fbm2(u * 2., v * 40., 3, s.wrapping_add(7));
        k *= 1. - 0.08 * (tyre - 0.5).max(0.) * 2.;
        [k, k * 0.99, k * 0.97]
    })
}

/// Asphalt, dark and fine, for the yard: tiles at 8 m.
pub fn asphalt(seed: u64) -> Texture {
    let n = 256u32;
    let s = seed as u32;
    paint(n, n, |x, y| {
        let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
        let k = 0.2
            + 0.06 * fbm2(u * 8., v * 8., 4, s)
            + 0.05 * noise2(u * 128., v * 128., s.wrapping_add(1));
        [k, k, k * 1.02]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_are_made_the_same_from_a_seed() {
        assert_eq!(container_wear(3).width(), 512);
        // Two textures of the same pixels are still two textures; the
        // pixels are what must repeat, and `paint` is a pure function of
        // its closure, which reads only the seed.
        let a = concrete(1);
        let b = concrete(1);
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    }
}
