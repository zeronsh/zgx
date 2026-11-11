//! A world's particle emitters (design-3d-aaa.md, 5.4): the `World` calls of
//! `crate::particles`.
use super::World;
use crate::particles::{
    Emitter, EmitterError, EmitterId, EmitterStatus, Force, Lit, Particles, Spawn, UNSETTLED,
};

impl World {
    /// Its emitters, made with the first call needing them; the world
    /// unsettled while one keeps it drawing.
    fn particles<T>(&self, f: impl FnOnce(&mut Particles) -> T) -> T {
        self.make_particles();
        let on_gpu = self.caps().is_none_or(|caps| caps.compute);
        let view = self.view.clone();
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        let particles = particles_of(inner);
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
        self.pool_room(emitter.max, None)?;
        let id = self.particles(|p| p.add(emitter));
        self.view.invalidate();
        Ok(id)
    }

    /// Whether a pool of `max` particles may be held, in place of
    /// `replacing`'s if it's given: one binding and dispatch on the device,
    /// the world's CPU particles where the CPU updates them, the memory cap.
    fn pool_room(&self, max: u32, replacing: Option<EmitterId>) -> Result<(), EmitterError> {
        let caps = self.caps();
        let on_gpu = caps.is_none_or(|caps| caps.compute);
        let most = crate::particles::most(caps);
        if on_gpu && max > most {
            return Err(EmitterError::TooMany { asked: max, most });
        }
        let replaced = replacing.and_then(|id| {
            Some(
                self.inner
                    .borrow()
                    .particles
                    .as_ref()?
                    .child(id)?
                    .emitter
                    .max,
            )
        });
        // On the CPU, the world's emitters together hold at most
        // `CPU_WORLD_PARTICLES`: its frames' work on the main thread is
        // bounded however many there are.
        let on_cpu = !on_gpu
            || self
                .inner
                .borrow()
                .particles
                .as_ref()
                .is_some_and(|p| p.on_cpu);
        if on_cpu {
            let held = self
                .inner
                .borrow()
                .particles
                .as_ref()
                .map_or(0, |p| p.cpu_particles())
                .saturating_sub(replaced.map_or(0, |m| m.min(crate::particles::CPU_PARTICLES)));
            let left = crate::particles::CPU_WORLD_PARTICLES.saturating_sub(held);
            if max.min(crate::particles::CPU_PARTICLES) > left {
                return Err(EmitterError::TooMany {
                    asked: max,
                    most: left,
                });
            }
        }
        self.make_particles();
        let view = self.view.clone();
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        let particles = particles_of(inner);
        let bytes = particles.pool_bytes(max, on_gpu);
        let freed = replaced.map_or(0, |m| particles.pool_bytes(m, on_gpu));
        let data = inner
            .data
            .get_or_insert_with(|| Box::new(crate::compute::Data::new(view)));
        let cap = data.cap;
        data.room(bytes.saturating_sub(freed))
            .map_err(|error| match error {
                crate::GpuDataError::OverBudget { asked, used, cap } => {
                    EmitterError::OverBudget { asked, used, cap }
                }
                _ => EmitterError::OverBudget {
                    asked: bytes,
                    used: 0,
                    cap,
                },
            })
    }

    /// Emitter `id` as it is now; `None` if it was removed.
    pub fn emitter(&self, id: EmitterId) -> Option<Emitter> {
        Some(
            self.inner
                .borrow()
                .particles
                .as_ref()?
                .child(id)?
                .emitter
                .clone(),
        )
    }

    /// Change emitter `id` to `emitter` from its next update: rate, spawn,
    /// lifetime, velocity, forces, collision, looks and its sub-emitter, its
    /// particles living on in its pool. A different `max` makes its pool
    /// anew (its particles gone), checked as `add_emitter` checks one.
    /// `emitter`'s `burst` isn't spawned: `burst` spawns more.
    pub fn set_emitter(&self, id: EmitterId, emitter: Emitter) -> Result<(), EmitterError> {
        let Some(max) = self.emitter(id).map(|e| e.max) else {
            return Ok(());
        };
        if emitter.max != max {
            self.pool_room(emitter.max, Some(id))?;
        }
        if self.particles(|p| p.set(id, emitter)) {
            self.view.invalidate();
        }
        Ok(())
    }

    /// `change` made to emitter `id`'s description, from its next update
    /// (its pool kept).
    fn change_emitter(&self, id: EmitterId, change: impl FnOnce(Emitter) -> Emitter) {
        if let Some(emitter) = self.emitter(id) {
            let max = emitter.max;
            let changed = change(emitter);
            debug_assert_eq!(changed.max, max);
            let _ = self.set_emitter(id, changed);
        }
    }
    /// How many particles emitter `id` spawns a second, from its next update.
    pub fn set_emitter_rate(&self, id: EmitterId, per_second: f32) {
        self.change_emitter(id, |e| e.rate(per_second));
    }
    /// Where emitter `id` spawns, and how fast.
    pub fn set_emitter_spawn(&self, id: EmitterId, spawn: Spawn) {
        self.change_emitter(id, |e| e.spawn(spawn));
    }
    /// How long emitter `id`'s particles born from now live.
    pub fn set_emitter_lifetime(&self, id: EmitterId, seconds: std::ops::Range<f32>) {
        self.change_emitter(id, |e| e.lifetime(seconds));
    }
    /// Emitter `id`'s particles' velocity at birth, from its next update.
    pub fn set_emitter_velocity(&self, id: EmitterId, base: [f32; 3], spread: f32) {
        self.change_emitter(id, |e| e.velocity(base, spread));
    }
    /// The forces moving emitter `id`'s particles, those living too.
    pub fn set_emitter_forces(&self, id: EmitterId, forces: &[Force]) {
        self.change_emitter(id, |mut e| {
            e.forces.clear();
            forces.iter().fold(e, |e, &force| e.force(force))
        });
    }
    /// The colour emitter `id`'s particles are born with.
    pub fn set_emitter_color(&self, id: EmitterId, rgba: [f32; 4]) {
        self.change_emitter(id, |e| e.color(rgba));
    }
    /// Emitter `id`'s colour over life, its living particles' too.
    pub fn set_emitter_color_over_life(&self, id: EmitterId, keys: [[f32; 4]; 4]) {
        self.change_emitter(id, |e| e.color_over_life(keys));
    }
    /// The size emitter `id`'s particles are born.
    pub fn set_emitter_size(&self, id: EmitterId, metres: std::ops::Range<f32>) {
        self.change_emitter(id, |e| e.size(metres));
    }
    /// Emitter `id`'s size over life, its living particles' too.
    pub fn set_emitter_size_over_life(&self, id: EmitterId, keys: [f32; 4]) {
        self.change_emitter(id, |e| e.size_over_life(keys));
    }
    /// Emitter `id` lit by the scene (`Emitter::lit`), or (`None`) light
    /// of its own, its living particles too.
    pub fn set_emitter_lit(&self, id: EmitterId, lit: Option<Lit>) {
        self.change_emitter(id, |mut e| {
            e.lit = lit.map(Lit::sanitized);
            e
        });
    }

    /// How an emitter is doing; `None` if it was removed.
    pub fn emitter_status(&self, id: EmitterId) -> Option<EmitterStatus> {
        let inner = self.inner.borrow();
        let live = inner.particles.as_ref()?.child(id)?;
        Some(if live.failed {
            EmitterStatus::Failed
        } else if live.over_cpu {
            EmitterStatus::OverCpuParticles
        } else {
            EmitterStatus::Running
        })
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

    /// Seconds its particles have been stepped: for tests.
    #[doc(hidden)]
    pub fn particle_time(&self) -> f64 {
        self.inner
            .borrow()
            .particles
            .as_ref()
            .map_or(0., |p| p.time)
    }

    /// The last frame's pools, each emitter's in the order they're updated:
    /// its slots and whether the CPU updated it. For tests.
    #[doc(hidden)]
    pub fn particle_pools(&self) -> Vec<(u32, bool)> {
        self.inner
            .borrow()
            .particles
            .as_ref()
            .map_or_else(Vec::new, |p| p.pools.clone())
    }

    /// Update particles on the CPU even where the device computes, as tier
    /// B does: for tests comparing the two.
    #[doc(hidden)]
    pub fn set_particles_on_cpu(&self, on: bool) {
        self.particles(|p| p.on_cpu = on);
        self.view.invalidate();
    }
}

/// `inner`'s particles (`World::make_particles` made them).
fn particles_of(inner: &mut super::Inner) -> &mut Particles {
    inner.particles.get_or_insert_with(Default::default)
}

impl World {
    /// Its particles made with its first emitter, their drawer registered:
    /// drawn with the meshes from then on (a world with none has no
    /// drawer).
    fn make_particles(&self) {
        if self.inner.borrow().particles.is_some() {
            return;
        }
        let particles = Box::<Particles>::default();
        let draw = particles.draw.clone();
        self.inner.borrow_mut().particles = Some(particles);
        self.add_scene_draw(draw);
    }
}
