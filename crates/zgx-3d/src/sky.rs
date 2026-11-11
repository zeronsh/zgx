//! A world's sky (feature `media`, `World::set_sky`): the light the air
//! scatters toward the eye from the world's sun (its first directional
//! light), a sun disc, the ground under the horizon and a deck of clouds,
//! drawn behind the meshes in the units its lights are in. The same model
//! gives the light reaching the ground (`Sky::sun_light`) and the
//! environment the meshes reflect and are lit by (`Sky::environment`).
use crate::{Environment, Light, Vec3, vec3};

/// A sky (`World::set_sky`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub(crate) turbidity: f32,
    pub(crate) ground: f32,
    pub(crate) sun_radius: f32,
    pub(crate) altitude: f32,
    pub(crate) clouds: Option<Clouds>,
}
impl Default for Sky {
    fn default() -> Self {
        Self::new()
    }
}
/// A deck of clouds (`Sky::clouds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    pub(crate) cover: f32,
    pub(crate) height: f32,
    pub(crate) size: f32,
    pub(crate) speed: f32,
    pub(crate) direction: [f32; 2],
    pub(crate) seed: f32,
}
impl Default for Clouds {
    fn default() -> Self {
        Self::new(0.4)
    }
}
impl Clouds {
    /// A deck over `cover` of the sky (0 to 1), 1,500 m up, still.
    pub fn new(cover: f32) -> Self {
        Self {
            cover: finite_or(cover, 0.).clamp(0., 1.),
            height: 1500.,
            size: 4000.,
            speed: 0.,
            direction: [1., 0.],
            seed: 0.,
        }
    }
    /// Its height above the sea, in metres.
    pub fn height(mut self, metres: f32) -> Self {
        self.height = finite_or(metres, 1500.).clamp(100., 12_000.);
        self
    }
    /// How large its billows are, in metres.
    pub fn size(mut self, metres: f32) -> Self {
        self.size = finite_or(metres, 4000.).clamp(10., 100_000.);
        self
    }
    /// It drifting at `speed` metres a second toward `direction` (x, z) on
    /// the frames' time: the world draws while it drifts. 0 (the default)
    /// keeps it still and lets the world rest.
    pub fn drifting(mut self, speed: f32, direction: [f32; 2]) -> Self {
        self.speed = finite_or(speed, 0.).clamp(0., 1000.);
        let [x, z] = direction.map(|v| finite_or(v, 0.));
        let length = (x * x + z * z).sqrt();
        self.direction = if length > 1e-6 {
            [x / length, z / length]
        } else {
            [1., 0.]
        };
        self
    }
    /// Another pattern of it (any number).
    pub fn seed(mut self, seed: f32) -> Self {
        self.seed = finite_or(seed, 0.).rem_euclid(1000.);
        self
    }
}
fn finite_or(v: f32, or: f32) -> f32 {
    if v.is_finite() { v } else { or }
}

impl Sky {
    /// A clear sky: turbidity 2, a ground reflecting 0.3, the sun half a
    /// degree across, the eye at the sea.
    pub fn new() -> Self {
        Self {
            turbidity: 2.,
            ground: 0.3,
            sun_radius: 0.25_f32.to_radians(),
            altitude: 0.,
            clouds: None,
        }
    }
    /// How hazy (1 the clearest air, 10 a heavy summer haze): more haze
    /// whitens the sky and widens the glow round the sun.
    pub fn turbidity(mut self, turbidity: f32) -> Self {
        self.turbidity = finite_or(turbidity, 2.).clamp(1., 20.);
        self
    }
    /// How much light the ground under the horizon reflects (0 to 1).
    pub fn ground(mut self, albedo: f32) -> Self {
        self.ground = finite_or(albedo, 0.3).clamp(0., 1.);
        self
    }
    /// The sun's disc, `degrees` across.
    pub fn sun_size(mut self, degrees: f32) -> Self {
        self.sun_radius = (finite_or(degrees, 0.5).clamp(0.05, 10.) * 0.5).to_radians();
        self
    }
    /// The world's origin `metres` above the sea (the eye's height adds
    /// its camera's).
    pub fn altitude(mut self, metres: f32) -> Self {
        self.altitude = finite_or(metres, 0.).clamp(-100., 50_000.);
        self
    }
    /// It with `clouds` (`None`: clear).
    pub fn clouds(mut self, clouds: Option<Clouds>) -> Self {
        self.clouds = clouds;
        self
    }
    /// Its numbers for the GPU, and how fast its clouds drift.
    pub(crate) fn uniform(&self) -> (crate::render::SkyUniform, f32) {
        let m = Model::of(self);
        let clouds = self.clouds.unwrap_or(Clouds {
            cover: 0.,
            ..Clouds::default()
        });
        (
            crate::render::SkyUniform {
                rayleigh: [m.air[0], m.air[1], m.air[2], m.haze],
                shape: [m.g, self.sun_radius, self.altitude, self.ground],
                clouds: [clouds.cover, clouds.height, clouds.size, 0.],
                drift: [clouds.direction[0], clouds.direction[1], clouds.seed, 0.],
            },
            if clouds.cover > 0. { clouds.speed } else { 0. },
        )
    }
    /// The sun as it reaches the ground through this sky's air: light going
    /// along `direction` (as `Light::Directional`'s), `intensity` its
    /// irradiance above the air. Low, it reddens and dims.
    pub fn sun_light(&self, direction: Vec3, intensity: f32) -> Light {
        let m = Model::of(self);
        let sun = (-direction).normalized();
        let eye = vec3(0., EARTH + self.altitude.max(1.), 0.);
        let t = m.transmittance_to_top(eye, sun);
        let lit = if ground_at(eye, sun) > 0. { [0.; 3] } else { t };
        let brightest = lit.iter().copied().fold(0., f32::max);
        if brightest < 1e-6 {
            // Set: none.
            return Light::Directional {
                direction,
                color: [1.; 3],
                intensity: 0.,
            };
        }
        Light::Directional {
            direction,
            color: lit.map(|c| c / brightest),
            intensity: intensity * brightest,
        }
    }
    /// The light all around under this sky, its clouds' included, for
    /// `World::set_environment`, the sun going `direction` with irradiance
    /// `intensity` (the sun's disc left to the directional light). Worked
    /// out on the CPU (about 256 by 128 directions, then prefiltered as
    /// `Environment::from_equirect` does): call it off the main thread, as
    /// the sun moves.
    pub fn environment(&self, direction: Vec3, intensity: f32) -> Environment {
        let (width, height) = (256u32, 128u32);
        let m = Model::of(self);
        let sun = (-direction).normalized();
        let e = [intensity; 3];
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            let theta = (y as f32 + 0.5) / height as f32 * std::f32::consts::PI;
            for x in 0..width {
                let phi = (x as f32 + 0.5) / width as f32 * std::f32::consts::TAU;
                // As `Environment::from_equirect` reads it.
                let d = vec3(
                    -theta.sin() * phi.sin(),
                    theta.cos(),
                    theta.sin() * phi.cos(),
                );
                rgb.extend(m.radiance(self, d, sun, e));
            }
        }
        Environment::from_equirect(width, height, &rgb).expect("its size")
    }
}

const EARTH: f32 = 6360e3;
const TOP: f32 = 6420e3;
const AIR_HEIGHT: f32 = 8000.;
const HAZE_HEIGHT: f32 = 1200.;
const OZONE: [f32; 3] = [0.65e-6, 1.881e-6, 0.085e-6];

/// The scattering model `sky.wgsl` draws, on the CPU: its numbers.
struct Model {
    /// Air's scattering per metre at sea level (rgb).
    air: [f32; 3],
    /// Haze's.
    haze: f32,
    /// Haze's forward scattering.
    g: f32,
}
impl Model {
    fn of(sky: &Sky) -> Self {
        Self {
            air: [5.802e-6, 13.558e-6, 33.1e-6],
            // Clear air (turbidity 1) to a heavy haze (10).
            haze: 2.0e-6 * sky.turbidity,
            g: 0.76 + 0.01 * sky.turbidity.min(10.),
        }
    }
    fn optical(&self, air: f32, haze: f32) -> [f32; 3] {
        std::array::from_fn(|i| self.air[i] * air + self.haze * 1.11 * haze + OZONE[i] * air)
    }
    fn extinction(&self, air: f32, haze: f32) -> [f32; 3] {
        self.optical(air, haze).map(|o| (-o).exp())
    }
    fn depth_to_top(&self, o: Vec3, d: Vec3) -> (f32, f32) {
        let r = o.length();
        let c = o.dot(d) / r;
        (column(r, c, AIR_HEIGHT), column(r, c, HAZE_HEIGHT))
    }
    fn transmittance_to_top(&self, o: Vec3, d: Vec3) -> [f32; 3] {
        let (air, haze) = self.depth_to_top(o, d);
        self.extinction(air, haze)
    }
    /// The light reaching the eye from along `d`: the air's, the ground's
    /// or the clouds' (as `sky.wgsl`'s, without the sun's disc).
    fn radiance(&self, sky: &Sky, d: Vec3, sun: Vec3, e: [f32; 3]) -> [f32; 3] {
        let o = vec3(0., EARTH + sky.altitude.max(1.), 0.);
        let hit = ground_at(o, d);
        let end = if hit > 0. { hit } else { leave(o, d, TOP) };
        // As `sky.wgsl`'s `scattered`.
        let steps = 16;
        let (mut air, mut haze) = (0., 0.);
        let (mut light_air, mut light_haze) = ([0.; 3], [0.; 3]);
        let mut from = 0.;
        for i in 1..=steps {
            let f = i as f32 / steps as f32;
            let to = end * f * f;
            let step = to - from;
            let p = o + d * (from + 0.5 * step);
            from = to;
            let h = p.length() - EARTH;
            let (a, z) = (
                (-h / AIR_HEIGHT).exp() * step,
                (-h / HAZE_HEIGHT).exp() * step,
            );
            let before = (air, haze);
            air += a;
            haze += z;
            if ground_at(p, sun) > 0. {
                continue;
            }
            let within = self.optical(a, z);
            let (ta, tz) = self.depth_to_top(p, sun);
            let t = self.extinction(before.0 + ta, before.1 + tz);
            for c in 0..3 {
                let kept = if within[c] < 1e-5 {
                    1.
                } else {
                    (1. - (-within[c]).exp()) / within[c]
                };
                light_air[c] += t[c] * kept * a;
                light_haze[c] += t[c] * kept * z;
            }
        }
        let c = d.dot(sun);
        let rayleigh = 3. / (16. * std::f32::consts::PI) * (1. + c * c);
        let mie = henyey_greenstein(c, self.g);
        let view = self.extinction(air, haze);
        let mut light: [f32; 3] = std::array::from_fn(|i| {
            e[i] * (light_air[i] * self.air[i] * rayleigh + light_haze[i] * self.haze * mie)
        });
        if hit > 0. {
            let p = o + d * hit;
            let up = p.normalized();
            let lit = self.transmittance_to_top(p, sun);
            let facing = up.dot(sun).max(0.);
            let ambient = 0.08 * (sun.y + 0.1).max(0.);
            for i in 0..3 {
                light[i] += (lit[i] * e[i] * facing + e[i] * ambient) * sky.ground
                    / std::f32::consts::PI
                    * view[i];
            }
        } else if let Some(clouds) = sky.clouds.filter(|c| c.cover > 0.)
            && d.y > 0.01
        {
            let at = (clouds.height - sky.altitude) / d.y;
            let (x, z) = (d.x * at, d.z * at);
            let density = cloud_density(&clouds, x, z, 5);
            if density > 0. {
                let toward = {
                    let (sx, sz) = (sun.x + 1e-4, sun.z + 1e-4);
                    let l = (sx * sx + sz * sz).sqrt();
                    (sx / l, sz / l)
                };
                let look = |f: f32| {
                    let (dx, dz) = (toward.0 * clouds.size * f, toward.1 * clouds.size * f);
                    cloud_density(&clouds, x + dx, z + dz, 3)
                };
                let along = look(0.08) + look(0.2);
                let through = (-along * 1.6).exp();
                let silver = henyey_greenstein(c, 0.65) * 4. * std::f32::consts::PI;
                let deck = vec3(0., EARTH + clouds.height, 0.);
                let sunlit = self.transmittance_to_top(deck, sun);
                let base = 0.35 + (1. - 0.35) * through;
                let rise = (clouds.height - sky.altitude).max(0.);
                let air_below = AIR_HEIGHT * (1. - (-rise / AIR_HEIGHT).exp()) / d.y;
                let haze_below = HAZE_HEIGHT * (1. - (-rise / HAZE_HEIGHT).exp()) / d.y;
                let seen = self.extinction(air_below, haze_below);
                let alpha = density * smoothstep(0.01, 0.15, d.y);
                for i in 0..3 {
                    let cloud =
                        sunlit[i] * e[i] * (0.22 * base + 0.12 * through * silver) + light[i] * 0.6;
                    let front =
                        light[i] * ((1. - seen[i]) / (1. - view[i]).max(1e-3)).clamp(0., 1.);
                    light[i] += (cloud * seen[i] + front - light[i]) * alpha;
                }
            }
        }
        light
    }
}
/// As `sky.wgsl`'s `column`: Schüler's closed form of the Chapman
/// function.
fn column(r: f32, c: f32, scale: f32) -> f32 {
    let x = EARTH / scale;
    let h = (r - EARTH) / scale;
    let k = (x + h).sqrt();
    if c >= 0. {
        return scale * k / (k * c + 1.) * (-h).exp();
    }
    let x0 = (1. - c * c).sqrt() * (x + h);
    scale * (2. * x0.sqrt() * (x - x0).exp() - k / (1. - k * c) * (-h).exp())
}
fn henyey_greenstein(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    (1. - g2) / (4. * std::f32::consts::PI * (1. + g2 - 2. * g * c).powf(1.5))
}
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn leave(o: Vec3, d: Vec3, r: f32) -> f32 {
    let b = o.dot(d);
    let c = o.dot(o) - r * r;
    let h = b * b - c;
    if h < 0. { -1. } else { -b + h.sqrt() }
}
fn ground_at(o: Vec3, d: Vec3) -> f32 {
    let b = o.dot(d);
    let c = o.dot(o) - EARTH * EARTH;
    let h = b * b - c;
    if h < 0. {
        return -1.;
    }
    let t = -b - h.sqrt();
    if t > 0. { t } else { -1. }
}
fn hash2(x: f32, y: f32) -> f32 {
    let (qx, qy) = ((x * 0.1031).fract(), (y * 0.1030).fract());
    let s = qx * (qy + 33.33) + qy * (qx + 33.33);
    let (rx, ry) = (qx + s, qy + s);
    ((rx + ry) * rx).fract()
}
fn noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3. - 2. * fx), fy * fy * (3. - 2. * fy));
    let a = hash2(ix, iy);
    let b = hash2(ix + 1., iy);
    let c = hash2(ix, iy + 1.);
    let d = hash2(ix + 1., iy + 1.);
    let ab = a + (b - a) * ux;
    let cd = c + (d - c) * ux;
    ab + (cd - ab) * uy
}
fn fbm(mut x: f32, mut y: f32, octaves: u32) -> f32 {
    let (mut sum, mut amplitude) = (0., 0.5);
    for _ in 0..octaves {
        sum += noise(x, y) * amplitude;
        x = x * 2.03 + 17.1;
        y = y * 2.03 + 9.7;
        amplitude *= 0.5;
    }
    sum
}
fn cloud_density(c: &Clouds, x: f32, z: f32, octaves: u32) -> f32 {
    let u = x / c.size + c.seed;
    let v = z / c.size + c.seed;
    smoothstep(1. - c.cover, 1. - c.cover + 0.35, fbm(u, v, octaves))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(sky: &Sky, d: Vec3, sun: Vec3) -> [f32; 3] {
        Model::of(sky).radiance(sky, d.normalized(), sun.normalized(), [1.; 3])
    }

    #[test]
    fn the_noon_sky_is_bluest_overhead_and_paler_toward_the_horizon() {
        let sky = Sky::new();
        let sun = vec3(0.3, 1., 0.2);
        let zenith = at(&sky, vec3(0., 1., 0.), sun);
        let horizon = at(&sky, vec3(0., 0.02, -1.), sun);
        assert!(zenith[2] > zenith[1] && zenith[1] > zenith[0], "{zenith:?}");
        let blue = |c: [f32; 3]| c[2] / c[0];
        assert!(blue(zenith) > blue(horizon), "{zenith:?} {horizon:?}");
        // Of the order a clear sky's radiance is for the sun's irradiance.
        assert!(zenith[2] > 0.005 && zenith[2] < 0.1, "{zenith:?}");
    }

    #[test]
    fn a_low_sun_reddens_the_horizon_under_it_and_the_light_it_gives() {
        let sky = Sky::new();
        let sun = vec3(0., 0.03, -1.);
        let under = at(&sky, vec3(0., 0.03, -1.), sun);
        assert!(under[0] > under[2], "{under:?}");
        let Light::Directional {
            color, intensity, ..
        } = sky.sun_light(-sun, 3.)
        else {
            unreachable!()
        };
        assert!(color[0] > color[1] && color[1] > color[2], "{color:?}");
        assert!(intensity < 3., "{intensity}");
        let Light::Directional {
            intensity: noon, ..
        } = sky.sun_light(vec3(0., -1., 0.), 3.)
        else {
            unreachable!()
        };
        assert!(noon > intensity && noon <= 3., "{noon}");
        // Below the horizon it gives none.
        let Light::Directional {
            intensity: night, ..
        } = sky.sun_light(vec3(0., 1., 0.), 3.)
        else {
            unreachable!()
        };
        assert_eq!(night, 0.);
    }

    #[test]
    fn hazier_air_is_whiter_and_clouds_cover_what_they_cover() {
        let sun = vec3(0.3, 1., 0.2);
        let d = vec3(0.3, 0.4, -1.);
        let clear = at(&Sky::new(), d, sun);
        let hazy = at(&Sky::new().turbidity(8.), d, sun);
        let blue = |c: [f32; 3]| c[2] / c[0];
        assert!(blue(hazy) < blue(clear), "{clear:?} {hazy:?}");
        let overcast = Sky::new().clouds(Some(Clouds::new(1.)));
        let clouded = at(&overcast, vec3(0., 1., 0.), sun);
        assert!(blue(clouded) < blue(at(&Sky::new(), vec3(0., 1., 0.), sun)));
        let none = Sky::new().clouds(Some(Clouds::new(0.)));
        assert_eq!(at(&none, d, sun), clear);
    }

    #[test]
    fn its_settings_are_kept_in_range() {
        let sky = Sky::new().turbidity(f32::NAN).ground(4.).sun_size(-1.);
        assert_eq!((sky.turbidity, sky.ground), (2., 1.));
        assert!(sky.sun_radius > 0.);
        let c = Clouds::new(2.).drifting(f32::INFINITY, [0., 0.]);
        assert_eq!((c.cover, c.speed, c.direction), (1., 0., [1., 0.]));
    }
}
