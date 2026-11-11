//! Global illumination (`gi` feature): irradiance probe volumes the app
//! places, filled with the light bouncing about the scene and read by
//! every lit mesh inside them for its diffuse light.
use super::{Settle, World};
use crate::math::Vec3;

impl Settle {
    /// Its probes are converging (`render/probes.rs`).
    pub(crate) const PROBES: u32 = super::Reason::Probes as u32;
}

/// The most volumes a world holds at once (`World::add_probes`).
pub const MAX_PROBE_VOLUMES: usize = 4;
/// The most probes a world's volumes hold together.
pub const MAX_PROBES: u32 = 8192;

/// A box of irradiance probes (`World::add_probes`): a grid of `counts`
/// probes spanning `min` to `max`, corner to corner. Each probe holds the
/// light arriving from every direction at its point (as diffuse light, an
/// octahedral map of 6 by 6 texels) and how far away the surfaces are
/// around it (so light doesn't leak through walls); a lit mesh inside the
/// box takes its diffuse light from the eight probes around each pixel.
///
/// Probes are filled by tracing rays where the device can (ray queries),
/// a budget of probes a frame, and kept: a world whose meshes and lights
/// hold still stops updating them once they've converged, and stops
/// drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeVolume {
    pub(crate) min: Vec3,
    pub(crate) max: Vec3,
    pub(crate) counts: [u32; 3],
    pub(crate) rays: u32,
    pub(crate) per_frame: u32,
}
impl ProbeVolume {
    /// Probes on a grid of `counts` (each at least 1; a count of 1 puts its
    /// probes midway) from `min` to `max`. 128 rays a probe, 64 probes
    /// updated a frame.
    pub fn new(min: impl Into<Vec3>, max: impl Into<Vec3>, counts: [u32; 3]) -> Self {
        let (a, b) = (min.into(), max.into());
        let finite =
            |v: Vec3| Vec3::from([v.x, v.y, v.z].map(|c| if c.is_finite() { c } else { 0. }));
        let (a, b) = (finite(a), finite(b));
        Self {
            min: Vec3::from([a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)]),
            max: Vec3::from([a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)]),
            counts: counts.map(|c| c.clamp(1, 64)),
            rays: 128,
            per_frame: 64,
        }
    }
    /// Rays traced from a probe each time it's updated (32 to 512, rounded
    /// to a multiple of 32): more, less noise and more cost.
    pub fn rays(mut self, rays: u32) -> Self {
        self.rays = rays.clamp(32, 512).div_ceil(32) * 32;
        self
    }
    /// Probes updated a frame (at least 1) while the volume is converging.
    pub fn per_frame(mut self, probes: u32) -> Self {
        self.per_frame = probes.max(1);
        self
    }
    /// Its probes, counted.
    pub fn probes(&self) -> u32 {
        self.counts.iter().product()
    }
    /// Probe `[x, y, z]`'s place.
    pub fn probe_position(&self, at: [u32; 3]) -> Vec3 {
        let axis = |i: usize, min: f32, max: f32| {
            let n = self.counts[i];
            if n <= 1 {
                (min + max) * 0.5
            } else {
                min + (max - min) * at[i].min(n - 1) as f32 / (n - 1) as f32
            }
        };
        Vec3::from([
            axis(0, self.min.x, self.max.x),
            axis(1, self.min.y, self.max.y),
            axis(2, self.min.z, self.max.z),
        ])
    }
}

/// What a world's last frame did with its probes (`World::probes_status`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProbesStatus {
    /// It has no volumes.
    NoVolumes,
    /// It has volumes and hasn't drawn a frame since.
    #[default]
    NotDrawn,
    /// Its probes are traced, and lit meshes inside its volumes take their
    /// diffuse light from them.
    Traced,
    /// Its device has no ray queries (`zgx_gpu::request_ray_queries` not
    /// called before the first window, the adapter has none, or Direct3D 12
    /// compiles with FXC: `zgx_gpu::ray_queries_status` says which): its
    /// volumes light nothing.
    NoRayQueries,
    /// It shades through clustered lights or the sun's cascades (the
    /// `lighting` feature), beside which probes don't yet light meshes:
    /// its volumes light nothing.
    BesideClustersOrCascades,
}

/// A volume in a world (`World::add_probes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProbesId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// A world's volumes, in the order added; a removed one's slot is reused.
#[derive(Default)]
pub(crate) struct Gi {
    pub(crate) volumes: [Option<(u32, ProbeVolume)>; MAX_PROBE_VOLUMES],
    generations: [u32; MAX_PROBE_VOLUMES],
    /// Counts changes to the volumes: the renderer lays out its probes
    /// again when it moves.
    pub(crate) layout: u64,
    /// What the last frame did with its probes (`World::probes_status`).
    pub(crate) status: std::cell::Cell<ProbesStatus>,
}
impl Gi {
    /// Whether it has any volume.
    pub(crate) fn any(&self) -> bool {
        self.volumes.iter().any(Option::is_some)
    }
    /// Its volumes, each with its slot.
    pub(crate) fn each(&self) -> impl Iterator<Item = (usize, &ProbeVolume)> {
        self.volumes
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().map(|(_, v)| (i, v)))
    }
}

impl World {
    /// Add a volume of irradiance probes: lit meshes inside it take their
    /// diffuse light from them (light bounced off the scene, the sky's
    /// included) in place of the hemisphere's or environment's. `None`
    /// where the world holds `MAX_PROBE_VOLUMES` already or the probes
    /// would pass `MAX_PROBES`.
    ///
    /// Probes are filled where the device traces rays (`zgx_gpu::
    /// request_ray_queries`, before the first window); elsewhere they stay
    /// dark and the volume lights nothing (`World::probes_traced` says
    /// which).
    pub fn add_probes(&self, volume: ProbeVolume) -> Option<ProbesId> {
        let mut id = None;
        self.change(|inner| {
            let gi = &mut inner.gi;
            let total: u32 = gi.each().map(|(_, v)| v.probes()).sum();
            if total + volume.probes() > MAX_PROBES {
                return false;
            }
            let Some(slot) = gi.volumes.iter().position(Option::is_none) else {
                return false;
            };
            let generation = gi.generations[slot];
            gi.volumes[slot] = Some((generation, volume));
            gi.layout += 1;
            gi.status.set(ProbesStatus::NotDrawn);
            id = Some(ProbesId {
                index: slot as u32,
                generation,
            });
            true
        });
        id
    }
    /// Remove a volume (nothing if it's gone).
    pub fn remove_probes(&self, id: ProbesId) {
        self.change(|inner| {
            let gi = &mut inner.gi;
            let slot = id.index as usize;
            match gi.volumes.get(slot) {
                Some(Some((generation, _))) if *generation == id.generation => {
                    gi.volumes[slot] = None;
                    gi.generations[slot] = gi.generations[slot].wrapping_add(1);
                    gi.layout += 1;
                    true
                }
                _ => false,
            }
        });
    }
    /// Whether the last frame drawn filled probes by tracing rays (false
    /// before a frame, without volumes, or where the device has no ray
    /// queries).
    pub fn probes_traced(&self) -> bool {
        self.inner.borrow().gi.status.get() == ProbesStatus::Traced
    }
    /// What the last frame drawn did with its probes, and where it didn't
    /// light by them, why.
    pub fn probes_status(&self) -> ProbesStatus {
        let inner = self.inner.borrow();
        if inner.gi.any() {
            inner.gi.status.get()
        } else {
            ProbesStatus::NoVolumes
        }
    }
}
