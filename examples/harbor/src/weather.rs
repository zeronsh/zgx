//! Rain: the demo's, written on the engine's particle emitters. Streaks
//! fall in a box that follows the camera, under gravity and a drag that
//! holds them near a raindrop's falling speed while the wind carries them;
//! where one reaches the ground it dies and splashes, a sub-emitter's
//! burst.
//!
//! What it can't do yet is in GAPS.md: drops collide with one plane (the
//! quay's), so over the water they stop three metres short of it; they
//! draw over every mesh, whatever is in front; and they take no light from
//! the scene, so their colour is given on the harbor's physical scale.
use crate::scene::UNIT;
use zgx_3d::{
    Blend, Collide, Emitter, EmitterError, EmitterId, Force, Response, Shape, Spawn, Vec3, World,
};

/// The rain's emitters.
#[derive(Clone, Copy, Debug)]
pub struct Rain {
    /// The falling drops.
    pub drops: EmitterId,
    /// Splashes where drops land.
    pub splashes: EmitterId,
}

/// Half the box rain falls in, across (m).
pub const REACH: f32 = 80.;
/// The box's top (m): drops born from the ground up to it.
pub const TOP: f32 = 70.;
/// Drops born a second.
pub const RATE: f32 = 45_000.;

/// What rain looks like by the sky's light: grey-blue, faint (its colour
/// is light it sends, in the harbor's units: GAPS.md, the waiting table).
const DROP: [f32; 4] = [0.11 * UNIT, 0.12 * UNIT, 0.14 * UNIT, 0.32];

impl Rain {
    /// Rain in `world`, falling with `wind` (m/s), or the emitters' error.
    pub fn new(world: &World, wind: [f32; 3]) -> Result<Self, EmitterError> {
        let splashes = world.add_emitter(
            Emitter::new(60_000)
                .spawn(Spawn::sphere([0., 0.02, 0.], 0.05))
                .lifetime(0.18..0.32)
                .velocity([0., 1.7, 0.], 0.9)
                .force(Force::Gravity)
                .collide(Collide::Plane {
                    height: 0.,
                    then: Response::Die,
                })
                .draw(Shape::Billboard, Blend::Alpha)
                .size(0.015..0.03)
                .color([DROP[0], DROP[1], DROP[2], 0.45])
                .color_over_life([[1.; 4], [1.; 4], [1., 1., 1., 0.6], [1., 1., 1., 0.]]),
        )?;
        let drops = world.add_emitter(
            Emitter::new(250_000)
                .spawn(Spawn::box_([-REACH, 1., -REACH], [REACH, TOP, REACH]).rate(RATE))
                // Long enough to reach the ground from the box's top.
                .lifetime(8.0..8.2)
                .velocity([0., -9., 0.], 0.6)
                // Gravity against drag toward the wind: about 9 m/s down,
                // carried along with the wind.
                .force(Force::Gravity)
                .force(Force::Drag(1.1))
                .force(Force::Wind(wind))
                .collide(Collide::Plane {
                    height: 0.,
                    then: Response::Die,
                })
                .on_hit(splashes, 2)
                .draw(Shape::Streak { length: 0.035 }, Blend::Alpha)
                .size(0.006..0.01)
                .color(DROP),
        )?;
        Ok(Self { drops, splashes })
    }

    /// The box rain falls in, moved over `camera` (its height stays).
    pub fn follow(&self, world: &World, camera: Vec3) {
        world.move_emitter(self.drops, [camera.x, 0., camera.z]);
    }

    /// Raining, or stopping (what has fallen finishes falling).
    pub fn set(&self, world: &World, on: bool) {
        world.set_emitting(self.drops, on);
    }
}
