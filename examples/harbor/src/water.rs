//! The water and the wet, the demo's own, written as custom materials on
//! zgx-3d's `MaterialShader`: `shaders/water.wgsl` on the sea (a grid
//! close in, for its waves to move, and the flat plane beyond), and
//! `shaders/wet.wgsl` as a surface layer over everything while it rains.
use crate::{
    scene::{EV, WATER_LEVEL},
    shapes::Builder,
};
use zgx_3d::{
    Material, MaterialShader, Object, ObjectId, Param, ShaderError, ShaderId, World, vec3,
};

/// The water's shaders, and the wet layer.
#[derive(Clone, Copy, Debug)]
pub struct Water {
    /// The sea's material.
    pub shader: ShaderId,
    /// The wet layer over every lit mesh.
    pub wet: Option<ShaderId>,
    /// The grid close in.
    pub near: ObjectId,
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

/// The near grid: its extent (x from, x to, z from, z to) and spacing.
const NEAR: (f32, f32, f32, f32, f32) = (-800., 800., -1560., 40., 4.);

/// The wave height the shader may reach, for culling and shadows.
const REACH: f32 = 0.45;

impl Water {
    /// The sea's shader on `far` (the plane beyond) and on a grid close in,
    /// and the wet layer, in `world`, the wind blowing `wind` (m/s).
    pub fn new(
        world: &World,
        far: ObjectId,
        wind: [f32; 3],
        parts: Parts,
    ) -> Result<Self, ShaderError> {
        let source = include_str!("shaders/water.wgsl");
        // Without refraction, for the bench to tell its cost: nothing
        // seen behind, so no picture of the scene is drawn for it.
        let source = if parts.refraction {
            source.to_owned()
        } else {
            source.replace("scene_color(s.screen_uv + slope * 0.03, 0.0)", "vec3(0.0)")
        };
        let shader = MaterialShader::new(source)
            .param("deep", Param::Color)
            .param("wind", Param::Vec2)
            .param("chop", Param::Float)
            .param("rain", Param::Float)
            .param("unexpose", Param::Float)
            .animated()
            .displaces_by(REACH);
        let shader = world.add_shader(if parts.refraction {
            shader.reads_scene()
        } else {
            shader
        })?;
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
        let material = world
            .object(far)
            .map_or(Material::rgb(0x18323a).with(0., 0.06), |o| o.material);
        let near = world.add(Object::mesh(b.build(), material).at([0., WATER_LEVEL, 0.]));
        world.set_shader(near, Some(shader));
        world.set_shader(far, Some(shader));
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
        let water = Self { shader, wet, near };
        world.set_param(shader, "deep", [0.006, 0.022, 0.026, 1.])?;
        world.set_param(shader, "wind", [wind[0], wind[2]])?;
        world.set_param(shader, "chop", 1.)?;
        if let Some(wet) = wet {
            world.set_param(wet, "ground", 0.)?;
        }
        water.set_rain(world, 0.);
        water.set_exposure(world, 0.);
        Ok(water)
    }

    /// How much it rains, 0 to 1: rings on the water, and everything wet.
    pub fn set_rain(&self, world: &World, rain: f32) {
        let _ = world.set_param(self.shader, "rain", rain);
        if let Some(wet) = self.wet {
            let _ = world.set_param(wet, "wet", rain);
        }
    }

    /// The panel's exposure, `stops` over the harbor's own: the water
    /// takes it off what it sees behind (`scene_color` is the scene as
    /// displayed; GAPS.md, G10).
    pub fn set_exposure(&self, world: &World, stops: f32) {
        let _ = world.set_param(self.shader, "unexpose", (EV - stops).exp2());
    }

    /// The near grid's triangles.
    pub fn triangles() -> usize {
        let (x0, x1, z0, z1, step) = NEAR;
        (((x1 - x0) / step) as usize) * (((z1 - z0) / step) as usize) * 2
    }
}
