//! The dawn sky as light: an equirectangular image made in code (no HDR
//! file), handed to `Environment::from_equirect`. The sun's position and
//! colour are here too, so the light and the sky agree.
use zgx_3d::{Environment, Vec3, vec3};

/// The sun's direction toward it (up is +Y), at `elevation` degrees above
/// the horizon and `azimuth` degrees from -Z (out over the water) toward +X.
pub fn sun_toward(elevation: f32, azimuth: f32) -> Vec3 {
    let (e, a) = (elevation.to_radians(), azimuth.to_radians());
    vec3(e.cos() * a.sin(), e.sin(), -e.cos() * a.cos()).normalized()
}

/// The sun's colour, linear, at `elevation` degrees: deep orange at the
/// horizon to warm white higher up (a shaping by hand, not a model of the
/// atmosphere: the physical sky is the engine's, later).
pub fn sun_color(elevation: f32) -> [f32; 3] {
    let t = (elevation / 25.).clamp(0., 1.);
    [1., 0.42 + 0.5 * t, 0.18 + 0.68 * t]
}

/// The sky's colour, linear, toward `d` (unit), the sun toward `sun`.
pub fn radiance(d: Vec3, sun: Vec3) -> [f32; 3] {
    let c = clear(d, sun);
    // Twilight: the whole sky dims as the sun goes under, through about
    // a thousandth with it 12 degrees down (night lit by lamps).
    let t = ((sun.y + 0.21) / 0.29).clamp(0., 1.);
    let dim = 0.0008 + 0.9992 * (t * t * (3. - 2. * t)).powi(3);
    c.map(|v| v * dim)
}

/// [`radiance`] before twilight dims it.
fn clear(d: Vec3, sun: Vec3) -> [f32; 3] {
    let up = d.y;
    let lit = (sun.y / 0.3).clamp(0., 1.);
    if up < 0. {
        // Below the horizon: the sea and far ground, dim and blue-grey,
        // brighter toward the sun's side.
        let toward = d.dot(sun).max(0.).powi(4);
        let k = 0.06 + 0.1 * toward;
        return [k * 1.1, k * 1.0, k * 1.05];
    }
    let zenith = [0.05 + 0.1 * lit, 0.09 + 0.16 * lit, 0.22 + 0.25 * lit];
    let horizon = [0.95, 0.55, 0.36];
    let away = [0.42, 0.42, 0.52];
    // Horizon colour: warm toward the sun, cool and pale away from it.
    let facing = (d.dot(sun) * 0.5 + 0.5).powf(2.);
    let band: Vec<f32> = (0..3)
        .map(|i| away[i] + (horizon[i] - away[i]) * facing)
        .collect();
    let blend = (1. - up).powf(5.);
    let mut c = [0.; 3];
    for i in 0..3 {
        c[i] = zenith[i] + (band[i] - zenith[i]) * blend;
    }
    // The glow round the sun, then its disc.
    let cos = d.dot(sun).clamp(-1., 1.);
    let glow = cos.max(0.).powf(64.) * 6. + cos.max(0.).powf(8.) * 0.8;
    let disc = if cos > 0.99985 && sun.y > -0.01 {
        400.
    } else {
        0.
    };
    let tint = sun_color(sun.y.asin().to_degrees());
    for i in 0..3 {
        c[i] += (glow + disc) * tint[i];
    }
    c
}

/// The sky with the sun toward `sun`, as an environment of `width` by
/// `width / 2` (linear RGB): 11–16 ms to prefilter, so make it off the
/// main thread.
pub fn dawn(sun: Vec3, width: u32) -> Environment {
    let height = width / 2;
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let d = equirect_direction(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            rgb.extend(radiance(d, sun));
        }
    }
    Environment::from_equirect(width, height, &rgb).expect("an equirect's size")
}

/// The direction at `(u, v)` of an equirect image as
/// `Environment::from_equirect` reads it: its centre column faces -Z, its
/// left edge +Z through -X, rows top (up) to bottom.
pub fn equirect_direction(u: f32, v: f32) -> Vec3 {
    let theta = v * std::f32::consts::PI;
    let phi = (u - 0.5) * std::f32::consts::TAU;
    vec3(
        theta.sin() * phi.sin(),
        theta.cos(),
        -theta.sin() * phi.cos(),
    )
}

/// The flat colour behind the world, sRGB: the sky at the horizon away
/// from the sun (the environment can't be drawn as the background yet:
/// GAPS.md, G2).
pub const BACKGROUND: u32 = 0x8c8a9c;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_laid_out_as_the_environment_reads_it() {
        // `from_equirect`'s documented frame: the centre faces -Z, the left
        // edge +Z through -X, the top row up.
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-3;
        assert!(close(equirect_direction(0.5, 0.5), vec3(0., 0., -1.)));
        assert!(close(equirect_direction(0.25, 0.5), vec3(-1., 0., 0.)));
        assert!(close(equirect_direction(0., 0.5), vec3(0., 0., 1.)));
        assert!(equirect_direction(0.3, 0.).y > 0.999);
    }

    #[test]
    fn the_sky_is_brightest_toward_the_sun() {
        let sun = sun_toward(8., 30.);
        let at = |d: Vec3| radiance(d, sun).iter().sum::<f32>();
        assert!(at(sun) > 100.);
        assert!(at(sun) > at(vec3(-sun.x, sun.y, -sun.z).normalized()) * 10.);
    }
}
