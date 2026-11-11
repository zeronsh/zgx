//! The ground: the quay's concrete apron and the yard's asphalt as one
//! plane with a material of the demo's own (`shaders/ground.wgsl`), its
//! maps made here at start. What makes it read is what's under the rain:
//! slabs with sealed expansion joints and cracks, worn asphalt, painted
//! markings chipped and worn, tyre rubber along the lanes, oil, and the
//! wet as one gradient from standing water through soaked and damp to dry,
//! set by a height mask, its roughness ramping with it.
use crate::rng::{Rng, hash2};
use zgx_3d::{
    Geometry, Material, MaterialShader, Object, ObjectId, Param, ShaderError, ShaderId, Texture,
    World,
};

/// The ground's extent: x from, x to, z from, z to (m). The quay's edge is
/// z = 0; the apron runs to [`APRON`], the yard beyond.
pub const EXTENT: (f32, f32, f32, f32) = (-320., 320., 0., 380.);
/// Where the apron's concrete gives way to the yard's asphalt (m).
pub const APRON: f32 = 60.;

/// The ground's parts in a world.
#[derive(Clone, Copy, Debug)]
pub struct Ground {
    /// The plane.
    pub mesh: ObjectId,
    /// Its material while it's dry or damp (reads no time).
    pub calm: ShaderId,
    /// Its material while it rains (rings in the standing water).
    pub raining: ShaderId,
}

/// Periodic value noise at `(x, y)` with period `p` cells, in `[0, 1)`.
fn pnoise(x: f32, y: f32, p: i32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3. - 2. * t);
    let (u, v) = (s(fx), s(fy));
    let h = |a: i32, b: i32| hash2(a.rem_euclid(p), b.rem_euclid(p), seed);
    let top = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * u;
    let bottom = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * u;
    top + (bottom - top) * v
}

/// `octaves` of [`pnoise`] over `(u, v)` in `[0, 1)`, `base` cells across
/// the first: periodic over the unit square.
fn pfbm(u: f32, v: f32, base: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut weight, mut total, mut cells) = (0., 1., 0., base);
    for o in 0..octaves {
        sum += pnoise(u * cells as f32, v * cells as f32, cells, seed.wrapping_add(o)) * weight;
        total += weight;
        weight *= 0.5;
        cells *= 2;
    }
    sum / total
}

/// Non-periodic `octaves` of value noise.
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    crate::rng::fbm2(x, y, octaves, seed)
}

fn texture(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Texture {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            for c in f(x, y) {
                pixels.push((c.clamp(0., 1.) * 255. + 0.5) as u8);
            }
        }
    }
    Texture::from_rgba8(width, height, pixels).expect("a texture's size")
}

/// The ground's large-scale mask over [`EXTENT`], 1024 by 608 texels
/// (about 0.6 m each): red the ground's height for water to stand in (low
/// where it's 0: dips, ruts along the lanes, ponds by the drains), green
/// how damp it stays, blue oil and stains, alpha wear.
pub fn mask(seed: u64) -> Texture {
    let (w, h) = (1024u32, 608u32);
    let s = seed as u32;
    let (x0, x1, z0, z1) = EXTENT;
    let mut rng = Rng::new(seed ^ 0x6d61);
    // Oil: blobs where trucks wait, near the lanes.
    let oil: Vec<(f32, f32, f32)> = (0..220)
        .map(|_| (rng.range(x0, x1), rng.range(z0 + 8., z1), rng.range(0.6, 2.6)))
        .collect();
    texture(w, h, |px, py| {
        let x = x0 + (px as f32 + 0.5) / w as f32 * (x1 - x0);
        let z = z0 + (py as f32 + 0.5) / h as f32 * (z1 - z0);
        // Height: broad dips and fine unevenness; ruts where wheels run
        // along the lanes; the apron falls gently toward drains every 32 m.
        let broad = fbm(x * 0.02, z * 0.02, 4, s);
        let fine = fbm(x * 0.15, z * 0.15, 3, s.wrapping_add(5));
        let lane = lane_distance(z);
        let rut = (-(lane * lane) / 4.).exp() * 0.25;
        let drain = if z < crate::ground::APRON {
            let d = ((x / 32.).fract() - 0.5).abs() * 32.;
            (1. - (d / 6.).min(1.)) * 0.2
        } else {
            0.
        };
        let height = (0.65 * broad + 0.35 * fine - rut - drain + 0.2).clamp(0., 1.);
        let damp = fbm(x * 0.05, z * 0.05, 3, s.wrapping_add(9));
        let stain = oil
            .iter()
            .map(|&(ox, oz, r)| {
                let d = ((x - ox).powi(2) + (z - oz).powi(2)).sqrt() / r;
                (1. - d).max(0.) * (0.5 + 0.5 * fbm(x * 0.8, z * 0.8, 2, s.wrapping_add(3)))
            })
            .fold(0., f32::max);
        let wear = (rut * 2.5 + 0.3 * fbm(x * 0.07, z * 0.07, 3, s.wrapping_add(11))).min(1.);
        [height, damp, stain, wear]
    })
}

/// Distance (m) along z to the nearest lane's centre: the yard's lanes run
/// along x between its block rows, and the apron's two traffic lanes.
pub fn lane_distance(z: f32) -> f32 {
    if z < APRON {
        // The apron's lanes, beside and behind the cranes' landside legs.
        return (z - 41.).abs().min((z - 52.).abs());
    }
    let pitch = YARD_BLOCK_Z;
    let first = YARD_FIRST_Z - pitch * 0.5;
    let k = ((z - first) / pitch).round();
    (z - (first + k * pitch)).abs()
}

/// The yard's block rows, as `scene::yard` lays them: the first block's
/// centre (z) and the pitch between rows.
pub const YARD_FIRST_Z: f32 = 76.;
/// The pitch between the yard's block rows (6 rows of containers and a
/// lane).
pub const YARD_BLOCK_Z: f32 = 6. * (crate::shapes::CONTAINER_WIDTH + 0.32) + 14.;
/// The pitch between the yard's blocks along x.
pub const YARD_BLOCK_X: f32 = 8. * (crate::shapes::CONTAINER_40 + 0.7) + 20.;

/// One concrete slab's detail, 1024 texels over 6 m, periodic: red cracks
/// (how dark), green whether a crack is sealed with tar, blue the
/// aggregate's height (for its normal), alpha the slab's tone.
pub fn slab(seed: u64) -> Texture {
    let n = 1024u32;
    let s = seed as u32;
    let mut cracks = vec![0f32; (n * n) as usize];
    let mut sealed = vec![0f32; (n * n) as usize];
    let mut rng = Rng::new(seed ^ 0x736c);
    // Cracks: wandering lines from an edge, some sealed.
    for c in 0..7 {
        let tar = c % 2 == 0;
        let (mut x, mut y) = (rng.range(0., n as f32), rng.range(0., n as f32));
        let mut angle = rng.range(0., std::f32::consts::TAU);
        for _ in 0..rng.range(300., 900.) as usize {
            angle += rng.range(-0.35, 0.35);
            x = (x + angle.cos() * 1.5).rem_euclid(n as f32);
            y = (y + angle.sin() * 1.5).rem_euclid(n as f32);
            let width: i32 = if tar { 3 } else { 1 };
            for dy in -width..=width {
                for dx in -width..=width {
                    let (px, py) = (
                        (x as i32 + dx).rem_euclid(n as i32) as usize,
                        (y as i32 + dy).rem_euclid(n as i32) as usize,
                    );
                    let i = py * n as usize + px;
                    let falloff = 1. - ((dx * dx + dy * dy) as f32).sqrt() / (width as f32 + 1.);
                    cracks[i] = cracks[i].max(falloff);
                    if tar {
                        sealed[i] = sealed[i].max(falloff);
                    }
                }
            }
        }
    }
    texture(n, n, |px, py| {
        let (u, v) = (px as f32 / n as f32, py as f32 / n as f32);
        let i = (py * n + px) as usize;
        let grain = 0.6 * pfbm(u, v, 96, 4, s) + 0.4 * pfbm(u, v, 384, 2, s.wrapping_add(4));
        let tone = pfbm(u, v, 3, 3, s.wrapping_add(8));
        [cracks[i], sealed[i], grain, tone]
    })
}

/// Numerals 0 to 9 in a stencil face, side by side, each 64 by 96 texels,
/// white on black in red: the yard's bay numbers.
pub fn digits() -> Texture {
    let (cw, ch) = (64u32, 96u32);
    // Seven segments, each a box (x0, y0, x1, y1) in a 64 by 96 cell, with
    // a stencil gap where a segment would join another.
    let seg = [
        (14., 8., 50., 18.),  // a: top
        (46., 14., 56., 46.), // b: top right
        (46., 50., 56., 82.), // c: bottom right
        (14., 78., 50., 88.), // d: bottom
        (8., 50., 18., 82.),  // e: bottom left
        (8., 14., 18., 46.),  // f: top left
        (14., 43., 50., 53.), // g: middle
    ];
    let on: [[bool; 7]; 10] = [
        [true, true, true, true, true, true, false],
        [false, true, true, false, false, false, false],
        [true, true, false, true, true, false, true],
        [true, true, true, true, false, false, true],
        [false, true, true, false, false, true, true],
        [true, false, true, true, false, true, true],
        [true, false, true, true, true, true, true],
        [true, true, true, false, false, false, false],
        [true, true, true, true, true, true, true],
        [true, true, true, true, false, true, true],
    ];
    texture(cw * 10, ch, |px, py| {
        let d = (px / cw) as usize;
        let (x, y) = ((px % cw) as f32 + 0.5, py as f32 + 0.5);
        let lit = seg
            .iter()
            .zip(on[d])
            .any(|(&(x0, y0, x1, y1), on)| on && x >= x0 && x <= x1 && y >= y0 && y <= y1);
        let v = if lit { 1. } else { 0. };
        [v, v, v, 1.]
    })
}

impl Ground {
    /// The ground's plane and its materials in `world`, its maps from
    /// `seed`.
    pub fn new(world: &World, seed: u64) -> Result<Self, ShaderError> {
        let (x0, x1, z0, z1) = EXTENT;
        let source = include_str!("shaders/ground.wgsl");
        let shader = |source: String, animated: bool| {
            let s = MaterialShader::new(source)
                .param("mask", Param::DataTexture)
                .param("slab", Param::DataTexture)
                .param("digits", Param::DataTexture)
                .param("extent", Param::Vec4)
                .param("yard", Param::Vec4)
                .param("wet", Param::Float)
                .param("rain", Param::Float);
            if animated { s.animated() } else { s }
        };
        let raining = world.add_shader(shader(source.to_owned(), true))?;
        let calm = world.add_shader(shader(source.replace("frame.time", "0.0"), false))?;
        let (mask, slab, digits) = (mask(seed), slab(seed), digits());
        for s in [calm, raining] {
            world.set_param(s, "mask", &mask)?;
            world.set_param(s, "slab", &slab)?;
            world.set_param(s, "digits", &digits)?;
            world.set_param(s, "extent", [x0, z0, x1 - x0, z1 - z0])?;
            world.set_param(s, "yard", [YARD_BLOCK_X, YARD_BLOCK_Z, YARD_FIRST_Z, APRON])?;
        }
        let mesh = world.add(
            Object::mesh(
                Geometry::plane(x1 - x0, z1 - z0),
                Material::rgb(0x808080).with(0., 0.7),
            )
            .at([(x0 + x1) * 0.5, 0., (z0 + z1) * 0.5]),
        );
        world.set_shader(mesh, Some(calm));
        let ground = Self {
            mesh,
            calm,
            raining,
        };
        ground.set_wet(world, 0., 0.);
        Ok(ground)
    }

    /// How wet the ground is (0 dry to 1 soaked, water standing in its
    /// dips) and whether it's raining now (rings in the water).
    pub fn set_wet(&self, world: &World, wet: f32, rain: f32) {
        let shader = if rain > 0. { self.raining } else { self.calm };
        world.set_shader(self.mesh, Some(shader));
        for s in [self.calm, self.raining] {
            let _ = world.set_param(s, "wet", wet);
            let _ = world.set_param(s, "rain", rain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_noise_wraps() {
        for i in 0..20 {
            let v = i as f32 / 20.;
            let (a, b) = (pfbm(0., v, 8, 4, 3), pfbm(1. - 1e-6, v, 8, 4, 3));
            assert!((a - b).abs() < 1e-2, "{a} {b}");
        }
    }

    #[test]
    fn lanes_lie_between_the_blocks() {
        // A lane's centre is half a block pitch from a block's centre.
        let z = YARD_FIRST_Z + YARD_BLOCK_Z * 0.5;
        assert!(lane_distance(z) < 1e-3);
        assert!(lane_distance(YARD_FIRST_Z) > 10.);
    }
}
