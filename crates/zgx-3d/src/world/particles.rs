//! A world's particle emitters (design-3d-aaa.md, 5.4): the `World` calls of
//! `crate::particles`.
use super::World;
use crate::particles::{Emitter, EmitterError, EmitterId, Particles, UNSETTLED};

impl World {
    /// Its emitters, made with the first call needing them; the world
    /// unsettled while one keeps it drawing.
    fn particles<T>(&self, f: impl FnOnce(&mut Particles) -> T) -> T {
        let on_gpu = self.caps().is_none_or(|caps| caps.compute);
        let view = self.view.clone();
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        let particles = inner.particles.get_or_insert_with(Default::default);
        let out = f(particles);
        let unsettled = particles.unsettled();
        // Its pools count with the app's data, under its cap.
        let bytes = particles.bytes(on_gpu);
        inner
            .data
            .get_or_insert_with(|| Box::new(crate::compute::Data::new(view)))
            .particle_bytes = bytes;
        inner.unsettle(UNSETTLED, unsettled);
        out
    }

    /// Add an emitter: it emits from the next frame, the world drawing
    /// while it emits or any of its particles may live. Where the device
    /// runs no compute (`caps().compute` false), its particles are updated
    /// on the CPU, at most `CPU_PARTICLES` of them.
    ///
    /// Refused past the most particles one pool holds on the view's device
    /// (`EmitterError::TooMany`: 1,677,721 on WebGPU's default limits, before
    /// a frame tells the device), and where its pool would take the app's GPU
    /// data past the world's cap (`set_memory_cap`).
    pub fn add_emitter(&self, emitter: Emitter) -> Result<EmitterId, EmitterError> {
        let caps = self.caps();
        let on_gpu = caps.is_none_or(|caps| caps.compute);
        let most = crate::particles::most(caps);
        if on_gpu && emitter.max > most {
            return Err(EmitterError::TooMany {
                asked: emitter.max,
                most,
            });
        }
        let room = {
            let view = self.view.clone();
            let mut inner = self.inner.borrow_mut();
            let inner = &mut *inner;
            let bytes = inner
                .particles
                .get_or_insert_with(Default::default)
                .pool_bytes(emitter.max, on_gpu);
            let data = inner
                .data
                .get_or_insert_with(|| Box::new(crate::compute::Data::new(view)));
            data.room(bytes).map_err(|error| match error {
                crate::GpuDataError::OverBudget { asked, used, cap } => {
                    EmitterError::OverBudget { asked, used, cap }
                }
                _ => EmitterError::OverBudget {
                    asked: bytes,
                    used: 0,
                    cap: data.cap,
                },
            })
        };
        room?;
        let id = self.particles(|p| p.add(emitter));
        self.view.invalidate();
        Ok(id)
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
