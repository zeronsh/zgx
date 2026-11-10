//! A world's particle emitters (design-3d-aaa.md, 5.4): the `World` calls of
//! `crate::particles`.
use super::World;
use crate::particles::{Emitter, EmitterId, Particles, UNSETTLED};

impl World {
    /// Its emitters, made with the first call needing them; the world
    /// unsettled while one keeps it drawing.
    fn particles<T>(&self, f: impl FnOnce(&mut Particles) -> T) -> T {
        let mut inner = self.inner.borrow_mut();
        let particles = inner.particles.get_or_insert_with(Default::default);
        let out = f(particles);
        let unsettled = particles.unsettled();
        inner.unsettle(UNSETTLED, unsettled);
        out
    }

    /// Add an emitter: it emits from the next frame, the world drawing
    /// while it emits or any of its particles may live. Where the device
    /// runs no compute (`caps().compute` false), its particles are updated
    /// on the CPU, at most `CPU_PARTICLES` of them.
    pub fn add_emitter(&self, emitter: Emitter) -> EmitterId {
        let id = self.particles(|p| p.add(emitter));
        self.view.invalidate();
        id
    }

    /// Take an emitter out, its particles with it.
    pub fn remove_emitter(&self, id: EmitterId) {
        if self.particles(|p| p.remove(id)) {
            self.view.invalidate();
        }
    }

    /// Whether an emitter spawns at its rate (it does when added). Stopped,
    /// its particles live out their lives, and then the world settles.
    pub fn set_emitting(&self, id: EmitterId, on: bool) {
        let changed = self.particles(|p| {
            p.get_mut(id)
                .is_some_and(|live| std::mem::replace(&mut live.emitting, on) != on)
        });
        if changed && on {
            self.view.invalidate();
        }
    }

    /// Spawn `count` particles from an emitter at once, in the next frame.
    pub fn burst(&self, id: EmitterId, count: u32) {
        if self.particles(|p| {
            p.get_mut(id)
                .map(|live| live.bursts = live.bursts.saturating_add(count))
                .is_some()
        }) {
            self.view.invalidate();
        }
    }

    /// Move where an emitter spawns by `offset` from where its `Spawn`
    /// says (particles already born stay where they are).
    pub fn move_emitter(&self, id: EmitterId, offset: [f32; 3]) {
        self.particles(|p| {
            if let Some(live) = p.get_mut(id) {
                live.offset = offset;
            }
        });
    }

    /// Whether a painter has made the particles' pipelines, so frames step
    /// them: for tests to wait on.
    #[doc(hidden)]
    pub fn particles_ready(&self) -> bool {
        self.inner
            .borrow()
            .particles
            .as_ref()
            .is_some_and(|p| p.ready)
    }

    /// Update particles on the CPU even where the device computes, as tier
    /// B does: for tests comparing the two.
    #[doc(hidden)]
    pub fn set_particles_on_cpu(&self, on: bool) {
        self.particles(|p| p.on_cpu = on);
        self.view.invalidate();
    }
}
