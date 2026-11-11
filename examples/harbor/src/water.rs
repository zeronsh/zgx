//! The water and the wet, the demo's own, on zgx-3d's public API:
//! - the sea's waves are a compute pass (`shaders/waves.wgsl`): a spectrum
//!   of wind waves summed each frame into a field over one 128 m tile, a
//!   `GpuTexture` the water's material reads;
//! - the water is a custom material (`shaders/water.wgsl`) on a grid close
//!   in, for its waves to move, and the flat plane beyond;
//! - the wet is a surface layer (`shaders/wet.wgsl`) over everything while
//!   it rains.
//!
//! A paused sea turns its pass off; its calm material reads no time, so
//! with nothing else moving the world rests.
use crate::{
    rng::Rng,
    scene::{EV, WATER_LEVEL},
    shapes::Builder,
};
use zgx_3d::{
    Access, BufferDesc, Compute, ComputeId, Format, GpuBuffer, GpuTexture, Material,
    MaterialShader, Object, ObjectId, Param, ShaderId, TextureDesc, World, vec3,
};

/// The field's side, texels, and the tile it covers, metres (as in the
/// shaders).
const SIDE: u32 = 512;
const TILE: f32 = 128.;
/// Waves in the spectrum.
const WAVES: usize = 64;

/// The water's parts in a world.
#[derive(Clone, Debug)]
pub struct Water {
    /// The sea's material while it doesn't rain: reads no time.
    pub calm: ShaderId,
    /// The sea's material while it rains: its rings move with time.
    pub raining: ShaderId,
    /// The wet layer over every lit mesh.
    pub wet: Option<ShaderId>,
    /// The grid close in, and the plane beyond.
    pub near: ObjectId,
    /// The plane beyond.
    pub far: ObjectId,
    /// The pass moving the sea's clock on.
    pub tick: ComputeId,
    /// The pass writing the wave field.
    pub waves: ComputeId,
    /// The wave field (kept: the handle owns it).
    pub field: GpuTexture,
    /// The waves' spectrum (kept: the handle owns it).
    pub spectrum: GpuBuffer,
    /// The sea's clock (kept).
    pub clock: GpuBuffer,
}

/// Which of the water's parts to make (all of them, but in the bench's
/// attribution arms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    /// The sea sees the scene behind it.
    pub refraction: bool,
    /// The wet layer over everything.
    pub wet: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Self {
            refraction: true,
            wet: true,
        }
    }
}

/// Why the water couldn't be made.
#[derive(Debug)]
pub enum WaterError {
    /// A material or layer was refused.
    Shader(zgx_3d::ShaderError),
    /// The wave field's data or pass was refused.
    Compute(String),
}
impl std::fmt::Display for WaterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shader(e) => write!(f, "{e}"),
            Self::Compute(e) => write!(f, "{e}"),
        }
    }
}
impl From<zgx_3d::ShaderError> for WaterError {
    fn from(e: zgx_3d::ShaderError) -> Self {
        Self::Shader(e)
    }
}

/// The near grid: its extent (x from, x to, z from, z to) and spacing.
const NEAR: (f32, f32, f32, f32, f32) = (-800., 800., -1560., 40., 4.);

/// A spectrum of `WAVES` wind waves on the tile's lattice: each its vector
/// (radians a metre, x and z), height (m) and phase. Directions spread
/// about `wind` (x, z) as cos², wavelengths from 1.5 m to the tile, heights
/// falling with wavelength's share as a wind sea's do, scaled so the sea's
/// significant height is `significant` m.
pub fn spectrum(seed: u64, wind: [f32; 2], significant: f32) -> Vec<[f32; 4]> {
    let mut rng = Rng::new(seed ^ 0x5ea);
    let w = {
        let l = (wind[0] * wind[0] + wind[1] * wind[1]).sqrt().max(1e-4);
        [wind[0] / l, wind[1] / l]
    };
    let lattice = std::f32::consts::TAU / TILE;
    let mut waves = Vec::with_capacity(WAVES);
    while waves.len() < WAVES {
        // A wavelength, more of the short than the long.
        let length = 1.5 * (TILE / 1.5).powf(rng.unit().powf(1.6));
        let angle = rng.range(-1.2, 1.2);
        // cos² spreading about the wind.
        if rng.unit() > angle.cos().powi(2) {
            continue;
        }
        let (s, c) = angle.sin_cos();
        let dir = [w[0] * c - w[1] * s, w[0] * s + w[1] * c];
        let k = std::f32::consts::TAU / length;
        // Onto the lattice: whole cycles across the tile.
        let (m, n) = (
            (dir[0] * k / lattice).round(),
            (dir[1] * k / lattice).round(),
        );
        if m == 0. && n == 0. {
            continue;
        }
        let kv = [m * lattice, n * lattice];
        let length = std::f32::consts::TAU / (kv[0] * kv[0] + kv[1] * kv[1]).sqrt();
        // A wind sea's heights rise with wavelength, less than in step.
        let height = length.powf(0.8);
        waves.push([kv[0], kv[1], height, rng.range(0., std::f32::consts::TAU)]);
    }
    // Significant height is about four times the sum's standard deviation,
    // sqrt(sum a² / 2).
    let sigma = (waves.iter().map(|w| w[2] * w[2]).sum::<f32>() * 0.5).sqrt();
    let scale = significant / (4. * sigma);
    for w in &mut waves {
        w[2] *= scale;
    }
    waves
}

impl Water {
    /// The sea's waves, its material on `far` (the plane beyond) and on a
    /// grid close in, and the wet layer, in `world`: the wind blowing
    /// `wind` (m/s), from `seed`.
    pub fn new(
        world: &World,
        far: ObjectId,
        wind: [f32; 3],
        seed: u64,
        parts: Parts,
    ) -> Result<Self, WaterError> {
        let compute = |e: &dyn std::fmt::Display| WaterError::Compute(e.to_string());
        let field = world
            .add_texture(TextureDesc::storage(SIDE, SIDE, Format::Rgba16Float))
            .map_err(|e| compute(&e))?;
        let waves_data = spectrum(seed, [wind[0], wind[2]], 0.5);
        // The most the waves can lift or drop the surface: all crests at
        // once. For culling and the shadows' fit.
        let reach: f32 = waves_data.iter().map(|w| w[2]).sum();
        let spectrum = world
            .add_buffer(BufferDesc::from_slice(&waves_data))
            .map_err(|e| compute(&e))?;
        // The sea's own clock, moved on by `tick` before `waves` reads it.
        let clock = world
            .add_buffer(BufferDesc::from_slice(&[[0f32; 4]]))
            .map_err(|e| compute(&e))?;
        let source = include_str!("shaders/waves.wgsl");
        let tick = world
            .add_compute(
                Compute::new(source)
                    .entry("tick")
                    .buffer("clock", &clock, Access::ReadWrite)
                    .threads([1, 1, 1])
                    .every_frame(),
            )
            .map_err(|e| compute(&e))?;
        let waves = world
            .add_compute(
                Compute::new(source)
                    .entry("waves")
                    .buffer("spectrum", &spectrum, Access::Read)
                    .buffer("now", &clock, Access::Read)
                    .texture("field", &field, Access::Write)
                    .threads([SIDE, SIDE, 1])
                    .every_frame(),
            )
            .map_err(|e| compute(&e))?;

        let source = include_str!("shaders/water.wgsl");
        // Without refraction, for the bench to tell its cost: nothing
        // seen behind, so no picture of the scene is drawn for it.
        let source = if parts.refraction {
            source.to_owned()
        } else {
            source.replace("scene_color(s.screen_uv + slope * 0.03, 0.0)", "vec3(0.0)")
        };
        let material = |source: String, animated: bool| {
            let shader = MaterialShader::new(source)
                .param("field", Param::GpuTexture)
                .param("deep", Param::Color)
                .param("wind", Param::Float)
                .param("rain", Param::Float)
                .param("unexpose", Param::Float)
                .displaces_by(reach);
            let shader = if animated { shader.animated() } else { shader };
            if parts.refraction {
                shader.reads_scene()
            } else {
                shader
            }
        };
        let raining = world.add_shader(material(source.clone(), true))?;
        // Calm: no rings, so no time read.
        let calm = world.add_shader(material(source.replace("frame.time", "0.0"), false))?;

        let (x0, x1, z0, z1, step) = NEAR;
        let (columns, rows) = (
            ((x1 - x0) / step) as usize + 1,
            ((z1 - z0) / step) as usize + 1,
        );
        let mut points = Vec::with_capacity(columns * rows);
        // Rows from the far edge toward the quay, so the grid's triangles
        // wind upward (`Builder::grid`).
        for r in 0..rows {
            let z = z1 - r as f32 * step;
            for c in 0..columns {
                let x = x0 + c as f32 * step;
                points.push((
                    vec3(x, 0., z),
                    [
                        c as f32 / (columns - 1) as f32,
                        r as f32 / (rows - 1) as f32,
                    ],
                ));
            }
        }
        let mut b = Builder::default();
        b.grid(rows, columns, &points);
        let stock = world
            .object(far)
            .map_or(Material::rgb(0x18323a).with(0., 0.06), |o| o.material);
        let near = world.add(Object::mesh(b.build(), stock).at([0., WATER_LEVEL, 0.]));
        // The plane beyond sits a metre under the grid's waves, so the two
        // never fight where they overlap.
        world.update(far, |o| o.transform.position.y = WATER_LEVEL - 1.);
        let wet = if parts.wet {
            Some(
                world.add_surface_layer(
                    MaterialShader::new(include_str!("shaders/wet.wgsl"))
                        .param("wet", Param::Float)
                        .param("ground", Param::Float),
                )?,
            )
        } else {
            None
        };
        for shader in [calm, raining] {
            world.set_param(shader, "field", &field)?;
            // Murky harbour water's body: a dark green-grey.
            world.set_param(shader, "deep", [0.008, 0.013, 0.012, 1.])?;
            let speed = (wind[0] * wind[0] + wind[2] * wind[2]).sqrt();
            world.set_param(shader, "wind", speed)?;
        }
        if let Some(wet) = wet {
            world.set_param(wet, "ground", 0.)?;
        }
        let water = Self {
            calm,
            raining,
            wet,
            near,
            far,
            tick,
            waves,
            field,
            spectrum,
            clock,
        };
        water.set_rain(world, 0.);
        water.set_exposure(world, 0.);
        Ok(water)
    }

    /// How much it rains, 0 to 1: rings on the water (its raining
    /// material), and everything wet.
    pub fn set_rain(&self, world: &World, rain: f32) {
        let shader = if rain > 0. { self.raining } else { self.calm };
        for mesh in [self.near, self.far] {
            world.set_shader(mesh, Some(shader));
        }
        let _ = world.set_param(self.raining, "rain", rain);
        if let Some(wet) = self.wet {
            let _ = world.set_param(wet, "wet", rain);
        }
    }

    /// Whether the sea moves: its passes run every frame, or not at all
    /// (its clock then stands, and goes on from there).
    pub fn set_moving(&self, world: &World, on: bool) {
        world.set_compute_enabled(self.tick, on);
        world.set_compute_enabled(self.waves, on);
    }

    /// The panel's exposure, `stops` over the harbor's own: what the water
    /// undoes on the scene behind it where that's the scene as displayed
    /// (no post chain); with the chain, `scene_color` is already the
    /// scene's light and the shader leaves it (`scene_is_light`).
    pub fn set_exposure(&self, world: &World, stops: f32) {
        let unexpose = (EV - stops).exp2();
        for shader in [self.calm, self.raining] {
            let _ = world.set_param(shader, "unexpose", unexpose);
        }
    }

    /// Whether its materials' pipelines are in.
    pub fn ready(&self, world: &World) -> bool {
        world.shader_ready(self.calm) || world.shader_ready(self.raining)
    }

    /// The near grid's triangles.
    pub fn triangles() -> usize {
        let (x0, x1, z0, z1, step) = NEAR;
        (((x1 - x0) / step) as usize) * (((z1 - z0) / step) as usize) * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spectrum_tiles_and_has_its_height() {
        let waves = spectrum(7, [2.5, 1.], 0.5);
        assert_eq!(waves.len(), WAVES);
        let lattice = std::f32::consts::TAU / TILE;
        for w in &waves {
            for k in [w[0], w[1]] {
                let cycles = k / lattice;
                assert!((cycles - cycles.round()).abs() < 1e-3, "{cycles}");
            }
        }
        let sigma = (waves.iter().map(|w| w[2] * w[2]).sum::<f32>() * 0.5).sqrt();
        assert!((4. * sigma - 0.5).abs() < 1e-3);
        // Down the wind more than across it.
        let along = waves.iter().filter(|w| w[0] * 2.5 + w[1] > 0.).count();
        assert!(along * 10 > WAVES * 8, "{along}");
    }
}
