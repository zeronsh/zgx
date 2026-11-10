//! Particles (design-3d-aaa.md, 5.4): emitters spawning into a pool of
//! their own, updated by a compute pass (or, where the device runs none, on
//! the CPU, up to [`CPU_PARTICLES`]), drawn as billboards or streaks.
//!
//! A pool is a ring: each frame spawns into the slots after the last
//! spawned, so a full pool replaces its oldest. Spawning, forces, collision
//! and ageing are stock modules set by the emitter's numbers, one shader for
//! every emitter. Drawn, the pool is the instances' vertex buffer: no
//! vertex shader reads a storage buffer, so the CPU's particles draw by the
//! same pipeline.
//!
//! An emitter keeps its world drawing while it emits or any of its
//! particles may live: until its last spawn is older than its longest life.
//! A world whose emitters are spent draws nothing.
mod cpu;
mod gpu;

pub(crate) use gpu::Device;
use std::ops::Range;

/// The most particles an emitter holds where they're updated on the CPU
/// (`Caps::compute` false): more are capped to it.
pub const CPU_PARTICLES: u32 = 16_384;

/// The `Settle` reason of a world's particles.
pub(crate) const UNSETTLED: u32 = 1 << 9;

/// The longest step a frame takes, in seconds: a world drawing again after
/// a pause doesn't spawn the pause's worth at once.
const MAX_STEP: f32 = 0.1;

/// Where an emitter's particles are born.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    region: Region,
    rate: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Region {
    Point([f32; 3]),
    Box([f32; 3], [f32; 3]),
    Sphere([f32; 3], f32),
    Cone([f32; 3], [f32; 3], f32),
}
impl Spawn {
    /// At a point.
    pub fn point(at: [f32; 3]) -> Self {
        Self::at(Region::Point(at))
    }
    /// Anywhere in the box between two corners.
    pub fn box_(min: [f32; 3], max: [f32; 3]) -> Self {
        Self::at(Region::Box(min, max))
    }
    /// Anywhere in a ball.
    pub fn sphere(centre: [f32; 3], radius: f32) -> Self {
        Self::at(Region::Sphere(centre, radius.max(0.)))
    }
    /// At `apex`, moving within `angle` radians of `axis` at the speed of
    /// the emitter's velocity.
    pub fn cone(apex: [f32; 3], axis: [f32; 3], angle: f32) -> Self {
        Self::at(Region::Cone(
            apex,
            axis,
            angle.clamp(0., std::f32::consts::PI),
        ))
    }
    fn at(region: Region) -> Self {
        Self { region, rate: 0. }
    }
    /// Born `per_second` a second of frame time while it emits.
    pub fn rate(mut self, per_second: f32) -> Self {
        self.rate = if per_second.is_finite() {
            per_second.max(0.)
        } else {
            0.
        };
        self
    }
}

/// What moves an emitter's particles.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Force {
    /// 9.81 m/s² down.
    Gravity,
    /// A constant acceleration.
    Constant([f32; 3]),
    /// Air: velocity pulled toward the wind's (still air without one) by
    /// this much a second.
    Drag(f32),
    /// The air's velocity drag pulls toward.
    Wind([f32; 3]),
    /// A swirling field: its strength (m/s²) and how tightly it turns
    /// (radians a metre).
    Swirl {
        /// Acceleration at its strongest.
        strength: f32,
        /// How tightly it turns.
        scale: f32,
    },
}

/// What a particle does on reaching a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Response {
    /// Bounces, keeping this much of its speed into the surface, and
    /// `friction` less of its speed along it.
    Bounce {
        /// Of its speed into the surface, kept (0 to 1).
        keep: f32,
        /// Of its speed along the surface, lost (0 to 1).
        friction: f32,
    },
    /// Stops where it hits, until its life ends.
    Stick,
    /// Ends there.
    Die,
}

/// What an emitter's particles collide with.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Collide {
    /// Nothing.
    #[default]
    None,
    /// The plane `y = height`, from above.
    Plane {
        /// Its height.
        height: f32,
        /// What they do there.
        then: Response,
    },
}

/// How a particle is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shape {
    /// A soft disc facing the camera, its size across.
    #[default]
    Billboard,
    /// A soft line along its motion: as long as it travels in `length`
    /// seconds, plus its size, its size across (rain, sparks).
    Streak {
        /// Seconds of motion its length shows.
        length: f32,
    },
}

/// How particles cover what's behind them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    /// Over it, by their opacity.
    #[default]
    Alpha,
    /// Adding their light to it (fire, sparks, glows).
    Additive,
}

/// An emitter to add (`World::add_emitter`): where and how fast particles
/// are born, how long they live, how they move and look.
#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub(crate) max: u32,
    pub(crate) spawn: Spawn,
    pub(crate) burst: u32,
    pub(crate) life: Range<f32>,
    pub(crate) velocity: ([f32; 3], f32),
    pub(crate) forces: Vec<Force>,
    pub(crate) collide: Collide,
    pub(crate) shape: Shape,
    pub(crate) blend: Blend,
    pub(crate) color: [f32; 4],
    pub(crate) colors: [[f32; 4]; 4],
    pub(crate) size: Range<f32>,
    pub(crate) sizes: [f32; 4],
}
impl Emitter {
    /// An emitter holding up to `max` particles at once, born at the
    /// origin, none a second, living a second, still, white, 5 cm across.
    pub fn new(max: u32) -> Self {
        Self {
            max: max.max(1),
            spawn: Spawn::point([0.; 3]),
            burst: 0,
            life: 1.0..1.0,
            velocity: ([0.; 3], 0.),
            forces: Vec::new(),
            collide: Collide::None,
            shape: Shape::Billboard,
            blend: Blend::Alpha,
            color: [1.; 4],
            colors: [[1.; 4]; 4],
            size: 0.05..0.05,
            sizes: [1.; 4],
        }
    }
    /// Where they're born, and how many a second.
    pub fn spawn(mut self, spawn: Spawn) -> Self {
        self.spawn = spawn;
        self
    }
    /// How many a second.
    pub fn rate(mut self, per_second: f32) -> Self {
        self.spawn = self.spawn.rate(per_second);
        self
    }
    /// So many born at once with the first frame (`World::burst` for
    /// more).
    pub fn burst(mut self, count: u32) -> Self {
        self.burst = count;
        self
    }
    /// Each lives a random time in `seconds`.
    pub fn lifetime(mut self, seconds: Range<f32>) -> Self {
        let start = seconds.start.max(1e-3);
        self.life = start..seconds.end.max(start);
        self
    }
    /// Each starts at `base` velocity (m/s), plus a random part up to
    /// `spread` in any direction.
    pub fn velocity(mut self, base: [f32; 3], spread: f32) -> Self {
        self.velocity = (base, spread.max(0.));
        self
    }
    /// A force moving them; forces add.
    pub fn force(mut self, force: Force) -> Self {
        self.forces.push(force);
        self
    }
    /// What they collide with.
    pub fn collide(mut self, collide: Collide) -> Self {
        self.collide = collide;
        self
    }
    /// How they're drawn and blended.
    pub fn draw(mut self, shape: Shape, blend: Blend) -> Self {
        self.shape = shape;
        self.blend = blend;
        self
    }
    /// Their colour when born: linear RGB and opacity.
    pub fn color(mut self, rgba: [f32; 4]) -> Self {
        self.color = rgba;
        self
    }
    /// Their colour over their life, multiplying the colour they're born
    /// with: at birth, a third, two thirds and the end of it.
    pub fn color_over_life(mut self, keys: [[f32; 4]; 4]) -> Self {
        self.colors = keys;
        self
    }
    /// Each is born a random size in `metres` across.
    pub fn size(mut self, metres: Range<f32>) -> Self {
        let start = metres.start.max(0.);
        self.size = start..metres.end.max(start);
        self
    }
    /// Their size over their life, as a factor of the size they're born:
    /// at birth, a third, two thirds and the end of it.
    pub fn size_over_life(mut self, keys: [f32; 4]) -> Self {
        self.sizes = keys;
        self
    }
    /// The longest a particle lives.
    fn longest(&self) -> f32 {
        self.life.end
    }
}

/// An emitter in a world (`World::add_emitter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EmitterId {
    index: u32,
    generation: u32,
}

/// An emitter and where it is in its frames.
pub(crate) struct Live {
    pub(crate) emitter: Emitter,
    /// Its spawn region moved by this much (`World::move_emitter`).
    pub(crate) offset: [f32; 3],
    pub(crate) emitting: bool,
    /// Born at once next frame (`burst`).
    pub(crate) bursts: u32,
    /// The part of a particle owed to the next frame's rate.
    owed: f32,
    /// The slot the next spawned is born in.
    pub(crate) head: u32,
    /// Frame time of the last frame it spawned in, if any.
    last_spawn: Option<f64>,
    /// Ids for the device's buffers: a new one when its pool changes.
    pub(crate) id: u64,
}
impl Live {
    /// Particles to spawn in a frame of `step` seconds at `time`: what it
    /// owes, at most a pool's worth. The ring's first slot is `head` before.
    pub(crate) fn step(&mut self, time: f64, step: f32, slots: u32) -> u32 {
        let rate = if self.emitting {
            self.emitter.spawn.rate
        } else {
            0.
        };
        let due = self.owed + rate * step;
        let whole = due.floor();
        self.owed = due - whole;
        let count = (whole as u64 + u64::from(std::mem::take(&mut self.bursts)))
            .min(u64::from(slots)) as u32;
        if count > 0 {
            self.last_spawn = Some(time);
        }
        count
    }
    /// Whether it keeps its world drawing at `time`: it emits, a burst is
    /// waiting, or a particle may live.
    fn unsettled(&self, time: f64) -> bool {
        (self.emitting && self.emitter.spawn.rate > 0.)
            || self.bursts > 0
            || self.last_spawn.is_some_and(|at| {
                time - at <= f64::from(self.emitter.longest()) + f64::from(MAX_STEP)
            })
    }
}

/// A world's emitters (`Inner::particles`), made with the first.
#[derive(Default)]
pub(crate) struct Particles {
    pub(crate) emitters: Vec<(u32, Option<Live>)>,
    next_id: u64,
    /// Update on the CPU even where the device computes (tests compare).
    pub(crate) on_cpu: bool,
    /// Seconds simulated: frame time, each frame's step at most
    /// `MAX_STEP`. Particles age by it, so whether any may live is read
    /// against it too.
    pub(crate) time: f64,
    /// Whether a painter has its pipelines, so frames step it.
    pub(crate) ready: bool,
    /// Frames stepped: what particles' random numbers are drawn from, so a
    /// run of frames repeats whatever came before them.
    pub(crate) steps: u32,
}
impl Particles {
    pub(crate) fn add(&mut self, emitter: Emitter) -> EmitterId {
        self.next_id += 1;
        let live = Live {
            bursts: emitter.burst,
            emitter,
            offset: [0.; 3],
            emitting: true,
            owed: 0.,
            head: 0,
            last_spawn: None,
            id: self.next_id,
        };
        let index = match self.emitters.iter().position(|(_, e)| e.is_none()) {
            Some(index) => index,
            None => {
                self.emitters.push((0, None));
                self.emitters.len() - 1
            }
        };
        let slot = &mut self.emitters[index];
        slot.0 = slot.0.wrapping_add(1);
        slot.1 = Some(live);
        EmitterId {
            index: index as u32,
            generation: slot.0,
        }
    }
    pub(crate) fn get_mut(&mut self, id: EmitterId) -> Option<&mut Live> {
        match self.emitters.get_mut(id.index as usize) {
            Some((generation, Some(live))) if *generation == id.generation => Some(live),
            _ => None,
        }
    }
    pub(crate) fn remove(&mut self, id: EmitterId) -> bool {
        match self.emitters.get_mut(id.index as usize) {
            Some((generation, live @ Some(_))) if *generation == id.generation => {
                *live = None;
                true
            }
            _ => false,
        }
    }
    pub(crate) fn live(&self) -> impl Iterator<Item = &Live> {
        self.emitters.iter().filter_map(|(_, e)| e.as_ref())
    }
    /// Bytes its pools hold on the device: as the GPU holds them, or as many
    /// as the CPU updates.
    pub(crate) fn bytes(&self, on_gpu: bool) -> u64 {
        let slots = |max: u32| {
            if on_gpu && !self.on_cpu {
                max
            } else {
                max.min(CPU_PARTICLES)
            }
        };
        self.live()
            .map(|live| u64::from(slots(live.emitter.max)) * cpu::PARTICLE_BYTES)
            .sum()
    }
    /// Whether any emitter keeps the world drawing.
    pub(crate) fn unsettled(&self) -> bool {
        self.live().any(|live| live.unsettled(self.time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_spawns_its_fraction_over_frames_and_a_burst_at_once() {
        let mut particles = Particles::default();
        let id = particles.add(Emitter::new(100).rate(30.).burst(5).lifetime(0.5..1.));
        let live = particles.get_mut(id).unwrap();
        // 30 a second at 60 frames a second: one every other frame.
        let counts: Vec<u32> = (0..6)
            .map(|frame| live.step(frame as f64 / 60., 1. / 60., 100))
            .collect();
        assert_eq!(counts, [5, 1, 0, 1, 0, 1]);
        // At most a pool's worth.
        live.bursts = 1000;
        assert_eq!(live.step(0.1, 1. / 60., 100), 100);
    }

    #[test]
    fn an_emitter_settles_its_longest_life_after_it_stops() {
        let mut particles = Particles::default();
        let id = particles.add(Emitter::new(10).burst(3).lifetime(0.2..0.5));
        assert!(particles.unsettled(), "a burst waits");
        particles.get_mut(id).unwrap().step(1., 0.016, 10);
        particles.time = 1.4;
        assert!(particles.unsettled(), "one may live");
        particles.time = 1. + 0.5 + f64::from(MAX_STEP) + 0.01;
        assert!(!particles.unsettled(), "none can");
        // Emitting at a rate keeps it unsettled; stopped, it isn't.
        let rain = particles.add(Emitter::new(10).rate(5.));
        assert!(particles.unsettled());
        particles.get_mut(rain).unwrap().emitting = false;
        assert!(!particles.unsettled());
    }
}
