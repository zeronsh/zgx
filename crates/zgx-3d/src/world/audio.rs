//! A world's sound (design-audio.md, 7), with the `audio` feature: emitters
//! on its objects, following their place in the world; the listener on its
//! camera (or an object); Doppler from how far each moved since the frame
//! before; occlusion by its meshes, from rays against the triangles picking
//! uses; and reverb zones the listener moves between.
//!
//! It all happens once a frame ([`World::update_audio`]: the world's view
//! calls it after each frame it draws). What changed since the last frame is
//! sent to the audio engine as ramps over the time between them, so a fast
//! pass doesn't step; a world where nothing moved sends nothing. The maths
//! is `zgx_audio::spatial`'s; each emitter is a voice of its own on the
//! world's bus, its distance, cone and occlusion in its gain, its pan by
//! equal power, Doppler in its pitch, and the air and the walls between in
//! its own low-pass ([`zgx_audio::Voice::set_cutoff`]).
use super::{Inner, World};
use crate::{ObjectId, Vec3, vec3};
use std::time::Instant;
use zgx_audio::effects::Reverb;
use zgx_audio::spatial::{self, Cone, Falloff};
use zgx_audio::{Audio, Bus, EffectHandle, Error, PanLaw, Play, Ramp, Sound, Voice};

/// Its reason to keep drawing (`Settle`): something moved last frame, so
/// the next must come, if only to say it stopped (Doppler back to 1).
pub(crate) const UNSETTLED: u32 = super::Reason::Audio as u32;

/// Rays a frame, unless set (`World::set_occlusion_rays`).
const RAYS: usize = 8;

/// What a mesh lets through unless set (`World::set_sound_transmission`):
/// a quarter of its level, -12 dB.
pub const TRANSMISSION: f32 = 0.25;

/// The longest step between frames a ramp or a velocity is taken over: a
/// world not drawn for a while doesn't glide for as long.
const LONGEST_STEP: f64 = 0.1;

/// How long occlusion and the zones take to change, at least.
const SETTLE: f64 = 0.05;

/// The cutoff, in hertz, at which a voice's low-pass is open.
const OPEN: f32 = 20_000.;

/// A sound on an object: it plays from where the object is, falling off
/// with distance, louder in its cone, its pitch moved by Doppler, dulled by
/// the air and by the meshes between it and the listener.
#[derive(Clone, Debug)]
pub struct Emitter3d {
    sound: Sound,
    looping: bool,
    gain: f32,
    falloff: Falloff,
    cone: Option<Cone>,
    doppler: f32,
    occlusion: bool,
    air: bool,
}

impl Emitter3d {
    /// `sound`, once, at its own level: falling off inversely from 1 m to
    /// 100 m, no cone, Doppler as the air would, occluded, dulled by the air.
    pub fn new(sound: &Sound) -> Emitter3d {
        Emitter3d {
            sound: sound.clone(),
            looping: false,
            gain: 1.,
            falloff: Falloff::inverse(1., 100.),
            cone: None,
            doppler: 1.,
            occlusion: true,
            air: true,
        }
    }
    /// Whether it plays round and round.
    pub fn looping(mut self, looping: bool) -> Emitter3d {
        self.looping = looping;
        self
    }
    /// Its own gain, before the rest.
    pub fn gain(mut self, gain: f32) -> Emitter3d {
        self.gain = gain;
        self
    }
    /// How it falls off with distance.
    pub fn falloff(mut self, falloff: Falloff) -> Emitter3d {
        self.falloff = falloff;
        self
    }
    /// Louder within `inner` degrees of where its object faces (its -z), at
    /// `outer_gain` past `outer`.
    pub fn cone(mut self, inner: f32, outer: f32, outer_gain: f32) -> Emitter3d {
        self.cone = Some(Cone::new(inner, outer, outer_gain));
        self
    }
    /// How much Doppler moves its pitch: 1 as the air would, 0 none.
    pub fn doppler(mut self, doppler: f32) -> Emitter3d {
        self.doppler = doppler;
        self
    }
    /// Whether the meshes between it and the listener dull it.
    pub fn occlusion(mut self, occlusion: bool) -> Emitter3d {
        self.occlusion = occlusion;
        self
    }
    /// Whether the air dulls it with distance (`zgx_audio::spatial::air_cutoff`).
    pub fn air(mut self, air: bool) -> Emitter3d {
        self.air = air;
        self
    }
}

/// An emitter in a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Emitter3dId {
    index: u32,
    generation: u32,
}

/// Where a world hears from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Listener {
    /// Its camera: where it is and where it looks (up, the world's).
    #[default]
    Camera,
    /// An object: its origin, its -z ahead and its y up.
    Object(ObjectId),
}

/// A reverb's settings (`zgx_audio::effects::Reverb`'s): its size (0.25 to
/// 2), decay in seconds to -60 dB, damping (0 to 1) and how much of it is
/// heard (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbPreset {
    /// How large the space sounds, 0.25 to 2.
    pub size: f32,
    /// Seconds to fall 60 dB.
    pub decay: f32,
    /// How much faster the highs die, 0 to 1.
    pub damping: f32,
    /// How much of it is heard, 0 to 1.
    pub wet: f32,
}

impl ReverbPreset {
    /// A small room: short and soft.
    pub const ROOM: ReverbPreset = ReverbPreset {
        size: 0.5,
        decay: 0.6,
        damping: 0.6,
        wet: 0.25,
    };
    /// A hall: long, bright.
    pub const HALL: ReverbPreset = ReverbPreset {
        size: 1.5,
        decay: 2.5,
        damping: 0.3,
        wet: 0.35,
    };
    /// A warehouse: large, long and hard.
    pub const WAREHOUSE: ReverbPreset = ReverbPreset {
        size: 2.,
        decay: 3.5,
        damping: 0.15,
        wet: 0.45,
    };
}

/// A volume with a reverb: the listener inside it hears its reverb,
/// fading in over `fade` metres as it comes toward it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbZone {
    min: Vec3,
    max: Vec3,
    preset: ReverbPreset,
    fade: f32,
}

impl ReverbZone {
    /// The box from `min` to `max`, a [`ReverbPreset::ROOM`], fading over
    /// 2 m.
    pub fn box_(min: impl Into<Vec3>, max: impl Into<Vec3>) -> ReverbZone {
        let (a, b) = (min.into(), max.into());
        ReverbZone {
            min: a.min(b),
            max: a.max(b),
            preset: ReverbPreset::ROOM,
            fade: 2.,
        }
    }
    /// Its reverb's settings.
    pub fn preset(mut self, preset: ReverbPreset) -> ReverbZone {
        self.preset = preset;
        self
    }
    /// Over how many metres outside it its reverb fades in.
    pub fn fade(mut self, metres: f32) -> ReverbZone {
        self.fade = metres.max(0.);
        self
    }
    /// How much a listener at `point` hears of it: 1 inside, falling in a
    /// straight line to 0 `fade` metres out.
    pub fn weight(&self, point: Vec3) -> f32 {
        let outside = (self.min - point).max(Vec3::ZERO) + (point - self.max).max(Vec3::ZERO);
        let distance = outside.length();
        if distance == 0. {
            1.
        } else if self.fade <= 0. {
            0.
        } else {
            (1. - distance / self.fade).max(0.)
        }
    }
}

/// A reverb zone in a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReverbZoneId {
    index: u32,
    generation: u32,
}

/// How an emitter is heard from where it is: what `zgx_audio::spatial`
/// makes of its distance, cone, motion and the air.
#[derive(Clone, Debug)]
struct Shape {
    falloff: Falloff,
    cone: Option<Cone>,
    gain: f32,
    doppler: f32,
    air: bool,
}

/// Its gain, pan, pitch and air's cutoff at `position`, facing `ahead`,
/// moving at `velocity`, heard by `listener`.
fn place(
    shape: &Shape,
    position: Vec3,
    ahead: Vec3,
    velocity: Vec3,
    listener: &spatial::Listener,
) -> [f32; 4] {
    let to_listener = [
        listener.position[0] - position.x,
        listener.position[1] - position.y,
        listener.position[2] - position.z,
    ];
    let distance = (listener_position(listener) - position).length();
    let cone = shape
        .cone
        .map_or(1., |cone| cone.gain(array(ahead), to_listener));
    let pitch = spatial::doppler(
        array(position),
        array(velocity),
        listener.position,
        listener.velocity,
        shape.doppler,
    );
    let cutoff = if shape.air {
        spatial::air_cutoff(distance)
    } else {
        OPEN
    };
    [
        shape.gain * shape.falloff.gain(distance) * cone,
        listener.pan(array(position)),
        pitch,
        cutoff,
    ]
}

fn listener_position(listener: &spatial::Listener) -> Vec3 {
    Vec3::from(listener.position)
}

/// An emitter as the world keeps it.
struct Placed {
    object: ObjectId,
    voice: Voice,
    shape: Shape,
    occlusion: bool,
    /// Where it was last frame, and which way it faced.
    last: Option<(Vec3, Vec3)>,
    velocity: Vec3,
    /// The occlusion's gain and cutoff, and the listener's and its place
    /// when last rayed.
    occluded: (f32, f32),
    rayed: Option<(Vec3, Vec3)>,
    /// Whether something changed since what was last sent.
    stale: bool,
}

/// Where the listener is and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Heard {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    velocity: Vec3,
}

/// A world's sound, kept in the world (`Inner::audio`).
pub(crate) struct Sound3d {
    audio: Audio,
    bus: Bus,
    reverb: Option<EffectHandle>,
    emitters: Vec<(u32, Option<Placed>)>,
    zones: Vec<(u32, Option<ReverbZone>)>,
    listener: Listener,
    heard: Option<Heard>,
    last_frame: Option<Instant>,
    /// Objects' transmission, by index and generation.
    transmission: Vec<(ObjectId, f32)>,
    rays: usize,
    /// The next emitter a ray is cast for.
    cursor: usize,
    /// Rays cast last frame (`World::audio_rays`).
    cast: usize,
    /// The reverb's settings last sent.
    mixed: Option<ReverbPreset>,
    /// The world's count of changes to its meshes when last rayed: a wall
    /// put up or moved between two ends that are still is rayed again.
    meshes: u64,
}

/// Worlds given an engine: each one's bus is named for it.
static WORLDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn array(v: Vec3) -> [f32; 3] {
    v.to_array()
}

impl Sound3d {
    /// Where `object` is and which way it faces, if it's there to hear.
    fn place_of(inner: &Inner, object: ObjectId) -> Option<(Vec3, Vec3)> {
        let index = inner.live(object.index, object.generation)?;
        let matrix = inner.world_of(index)?;
        let position = matrix.project(Vec3::ZERO);
        let ahead = (matrix.project(vec3(0., 0., -1.)) - position).normalized();
        Some((position, ahead))
    }

    /// The listener now, its velocity from where it was `step` seconds ago.
    fn hear(&self, inner: &Inner, step: f64) -> Heard {
        let (position, forward, up) = match self.listener {
            Listener::Object(id) => inner
                .live(id.index, id.generation)
                .and_then(|index| inner.world_of(index))
                .map(|m| {
                    let p = m.project(Vec3::ZERO);
                    let forward = (m.project(vec3(0., 0., -1.)) - p).normalized();
                    let up = (m.project(Vec3::Y) - p).normalized();
                    (p, forward, up)
                })
                .unwrap_or((Vec3::ZERO, vec3(0., 0., -1.), Vec3::Y)),
            Listener::Camera => {
                let camera = &inner.camera;
                let forward = (camera.target - camera.position).normalized();
                (camera.position, forward, Vec3::Y)
            }
        };
        let velocity = match self.heard {
            Some(last) if step > 0. => (position - last.position) * (1. / step as f32),
            _ => Vec3::ZERO,
        };
        Heard {
            position,
            forward,
            up,
            velocity,
        }
    }

    /// What the meshes between `from` and `to` let through: each one met
    /// multiplies the gain by its transmission and the cutoff by its
    /// transmission to the power 1.5 (a quarter: -12 dB, 2.5 kHz).
    /// `own` (the emitter's object) and `listener` (its, if it has one)
    /// don't count.
    fn occlusion(&self, inner: &Inner, from: Vec3, to: Vec3, own: ObjectId) -> (f32, f32) {
        let listener = match self.listener {
            Listener::Object(id) => Some(id.index as usize),
            Listener::Camera => None,
        };
        let (mut gain, mut cutoff) = (1f32, OPEN);
        for index in 0..inner.objects.len() {
            let slot = &inner.objects[index];
            let Some(object) = &slot.value else { continue };
            let Some(geometry) = &object.geometry else {
                continue;
            };
            if index == own.index as usize || Some(index) == listener {
                continue;
            }
            let id = ObjectId {
                index: index as u32,
                generation: slot.generation,
            };
            let through = self.transmission_of(id);
            if through >= 1. {
                continue;
            }
            let Some(matrix) = inner.world_of(index) else {
                continue;
            };
            if crate::pick::meet(geometry, matrix, from, to).is_some() {
                gain *= through;
                cutoff *= through * through.sqrt();
            }
        }
        (gain, cutoff.max(100.))
    }

    fn transmission_of(&self, id: ObjectId) -> f32 {
        self.transmission
            .iter()
            .find(|(object, _)| *object == id)
            .map_or(TRANSMISSION, |(_, t)| *t)
    }

    /// The zones' reverb for a listener at `point`: their settings weighted
    /// by how much it hears of each; its wet is the most it hears of any,
    /// times their weighted wet.
    fn zone_mix(&self, point: Vec3) -> ReverbPreset {
        let mut total = 0f32;
        let mut sum = [0f32; 4];
        let mut most = 0f32;
        for zone in self.zones.iter().filter_map(|(_, z)| z.as_ref()) {
            let w = zone.weight(point);
            if w > 0. {
                let p = zone.preset;
                for (s, v) in sum.iter_mut().zip([p.size, p.decay, p.damping, p.wet]) {
                    *s += w * v;
                }
                total += w;
                most = most.max(w);
            }
        }
        if total == 0. {
            return ReverbPreset {
                wet: 0.,
                ..ReverbPreset::ROOM
            };
        }
        let [size, decay, damping, wet] = sum.map(|s| s / total);
        ReverbPreset {
            size,
            decay,
            damping,
            wet: wet * most,
        }
    }

    /// One frame at `time`: sends what moved. Returns whether anything
    /// moved, so another frame must follow.
    fn frame(&mut self, inner: &Inner, time: Instant) -> bool {
        let step = self
            .last_frame
            .map_or(0., |last| {
                time.saturating_duration_since(last).as_secs_f64()
            })
            .min(LONGEST_STEP);
        self.last_frame = Some(time);
        let heard = self.hear(inner, step);
        let listener_moved = self.heard != Some(heard);
        self.heard = Some(heard);
        let listening = spatial::Listener {
            position: array(heard.position),
            forward: array(heard.forward),
            up: array(heard.up),
            velocity: array(heard.velocity),
        };
        let ramp = Ramp::seconds(step.max(0.));
        let mut moving = heard.velocity != Vec3::ZERO;
        // Each emitter's place and velocity.
        for (_, slot) in self.emitters.iter_mut() {
            let Some(placed) = slot else { continue };
            let Some((position, ahead)) = Self::place_of(inner, placed.object) else {
                if inner
                    .live(placed.object.index, placed.object.generation)
                    .is_none()
                {
                    // Its object is gone: so is it.
                    let _ = placed.voice.stop();
                    *slot = None;
                } else if placed.last.take().is_some() {
                    // Hidden: silent until it shows again.
                    let _ = placed.voice.set_gain(0., Ramp::seconds(SETTLE));
                }
                continue;
            };
            let velocity = match placed.last {
                Some((last, _)) if step > 0. => (position - last) * (1. / step as f32),
                _ => Vec3::ZERO,
            };
            if placed.last != Some((position, ahead)) || placed.velocity != velocity {
                placed.stale = true;
            }
            moving |= velocity != Vec3::ZERO;
            placed.last = Some((position, ahead));
            placed.velocity = velocity;
        }
        // Rays, for those whose ends moved, or all when a mesh changed,
        // round-robin within the budget.
        if inner.meshes != self.meshes {
            self.meshes = inner.meshes;
            for placed in self.emitters.iter_mut().filter_map(|(_, p)| p.as_mut()) {
                placed.rayed = None;
            }
        }
        self.cast = 0;
        let count = self.emitters.len();
        let start = self.cursor;
        let mut looked = 0;
        while looked < count && self.cast < self.rays {
            let index = (start + looked) % count;
            looked += 1;
            let Some(placed) = &self.emitters[index].1 else {
                continue;
            };
            let Some((position, _)) = placed.last else {
                continue;
            };
            let ends = (heard.position, position);
            if !placed.occlusion || placed.rayed == Some(ends) {
                continue;
            }
            let occluded = self.occlusion(inner, heard.position, position, placed.object);
            self.cast += 1;
            let placed = self.emitters[index].1.as_mut().expect("looked at");
            placed.rayed = Some(ends);
            if placed.occluded != occluded {
                placed.occluded = occluded;
                placed.stale = true;
            }
            self.cursor = (index + 1) % count.max(1);
        }
        // What changed, sent.
        for placed in self.emitters.iter_mut().filter_map(|(_, p)| p.as_mut()) {
            let Some((position, ahead)) = placed.last else {
                continue;
            };
            if !placed.stale && !listener_moved {
                continue;
            }
            let [level, pan, pitch, air] =
                place(&placed.shape, position, ahead, placed.velocity, &listening);
            let (gain, cutoff) = placed.occluded;
            // Occlusion moves no faster than `SETTLE`: a wall slides in.
            let slow = Ramp::seconds(step.max(SETTLE));
            let sent = placed.voice.set_gain(level * gain, slow).is_ok()
                && placed.voice.set_pan(pan, ramp).is_ok()
                && placed.voice.set_pitch(pitch, ramp).is_ok()
                && placed.voice.set_cutoff(air.min(cutoff), slow).is_ok();
            // Not all of it got through (the queue was full): again next
            // frame.
            placed.stale = !sent;
        }
        // The zones' reverb, where the listener is.
        if let Some(reverb) = &self.reverb {
            let mix = self.zone_mix(heard.position);
            if self.mixed != Some(mix) {
                let slow = Ramp::seconds(step.max(SETTLE));
                let sent = reverb.set(Reverb::SIZE, mix.size, slow).is_ok()
                    && reverb.set(Reverb::DECAY, mix.decay, slow).is_ok()
                    && reverb.set(Reverb::DAMPING, mix.damping, slow).is_ok()
                    && reverb.set(Reverb::WET, mix.wet, slow).is_ok();
                self.mixed = sent.then_some(mix);
            }
        }
        moving
    }
}

impl Sound3d {
    fn add(
        &mut self,
        inner: &Inner,
        object: ObjectId,
        emitter: Emitter3d,
    ) -> Result<Emitter3dId, Error> {
        let heard = self.hear(inner, 0.);
        let listener = spatial::Listener {
            position: array(heard.position),
            forward: array(heard.forward),
            up: array(heard.up),
            velocity: [0.; 3],
        };
        let shape = Shape {
            falloff: emitter.falloff,
            cone: emitter.cone,
            gain: emitter.gain,
            doppler: emitter.doppler,
            air: emitter.air,
        };
        let mut play = Play::new()
            .bus(&self.bus)
            .looping(emitter.looping)
            .law(PanLaw::EqualPower);
        // Placed where it is now, so it starts there.
        let at = Self::place_of(inner, object);
        play = match at {
            Some((position, ahead)) => {
                let [gain, pan, pitch, cutoff] =
                    place(&shape, position, ahead, Vec3::ZERO, &listener);
                play.gain(gain).pan(pan).pitch(pitch).cutoff(cutoff)
            }
            None => play.gain(0.),
        };
        let voice = self.audio.play(&emitter.sound, play)?;
        let placed = Placed {
            object,
            voice,
            shape,
            occlusion: emitter.occlusion,
            last: at,
            velocity: Vec3::ZERO,
            occluded: (1., OPEN),
            rayed: None,
            stale: false,
        };
        let (index, generation) = claim(&mut self.emitters, placed);
        Ok(Emitter3dId { index, generation })
    }
}

/// One frame of `world`'s sound at `time`, if it has any: its view calls
/// this after each frame it draws.
pub(crate) fn frame(world: &std::cell::RefCell<Inner>, time: Instant) {
    let mut inner = world.borrow_mut();
    let Some(mut sound) = inner.audio.take() else {
        return;
    };
    let moving = sound.frame(&inner, time);
    inner.audio = Some(sound);
    inner.unsettle(UNSETTLED, moving);
}

impl World {
    /// Gives it sound through `audio`: its emitters play on a bus of their
    /// own under `audio`'s master, which its reverb zones' reverb is on.
    /// Again, with another engine, starts over: emitters added before are
    /// stopped.
    pub fn set_audio(&self, audio: &Audio) -> Result<(), Error> {
        let n = WORLDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bus = audio.add_bus(&format!("3d world {n}"), &audio.master())?;
        let mut inner = self.inner.borrow_mut();
        if let Some(old) = inner.audio.take() {
            for placed in old.emitters.into_iter().filter_map(|(_, p)| p) {
                let _ = placed.voice.stop();
            }
        }
        inner.audio = Some(Box::new(Sound3d {
            audio: audio.clone(),
            bus,
            reverb: None,
            emitters: Vec::new(),
            zones: Vec::new(),
            listener: Listener::Camera,
            heard: None,
            last_frame: None,
            transmission: Vec::new(),
            rays: RAYS,
            cursor: 0,
            cast: 0,
            mixed: None,
            meshes: 0,
        }));
        Ok(())
    }

    /// Plays `emitter` on `object`: from where the object is, from now.
    /// Refused without an engine (`set_audio`, `Error::Effect`), or if the
    /// engine refuses the play.
    pub fn add_emitter3d(
        &self,
        object: ObjectId,
        emitter: Emitter3d,
    ) -> Result<Emitter3dId, Error> {
        let mut inner = self.inner.borrow_mut();
        let Some(mut sound) = inner.audio.take() else {
            return Err(Error::Effect("no engine: World::set_audio first".into()));
        };
        let added = sound.add(&inner, object, emitter);
        inner.audio = Some(sound);
        drop(inner);
        // Its occlusion is found by the next frame.
        self.view.invalidate();
        added
    }

    /// Stops emitter `id`, with a short fade.
    pub fn remove_emitter3d(&self, id: Emitter3dId) {
        let mut inner = self.inner.borrow_mut();
        let Some(sound) = inner.audio.as_mut() else {
            return;
        };
        if let Some((generation, slot)) = sound.emitters.get_mut(id.index as usize)
            && *generation == id.generation
            && let Some(placed) = slot.take()
        {
            let _ = placed.voice.stop();
        }
    }

    /// Where it hears from: its camera, the default, or an object.
    pub fn set_listener(&self, listener: Listener) {
        let mut inner = self.inner.borrow_mut();
        if let Some(sound) = inner.audio.as_mut() {
            sound.listener = listener;
        }
        drop(inner);
        self.view.invalidate();
    }

    /// Adds a reverb zone: the listener in it, or coming toward it, hears
    /// its reverb on every emitter. The first one puts the reverb on the
    /// world's bus.
    pub fn add_reverb_zone(&self, zone: ReverbZone) -> Result<ReverbZoneId, Error> {
        let mut inner = self.inner.borrow_mut();
        let Some(sound) = inner.audio.as_mut() else {
            return Err(Error::Effect("no engine: World::set_audio first".into()));
        };
        if sound.reverb.is_none() {
            let p = zone.preset;
            let reverb = Reverb::new(p.size, p.decay).damping(p.damping).mix(0., 1.);
            sound.reverb = Some(sound.bus.add_effect(reverb)?);
            sound.mixed = None;
        }
        let (index, generation) = claim(&mut sound.zones, zone);
        sound.mixed = None;
        drop(inner);
        self.view.invalidate();
        Ok(ReverbZoneId { index, generation })
    }

    /// Takes reverb zone `id` away.
    pub fn remove_reverb_zone(&self, id: ReverbZoneId) {
        let mut inner = self.inner.borrow_mut();
        if let Some(sound) = inner.audio.as_mut()
            && let Some((generation, slot)) = sound.zones.get_mut(id.index as usize)
            && *generation == id.generation
        {
            *slot = None;
            sound.mixed = None;
        }
        drop(inner);
        self.view.invalidate();
    }

    /// How much of a sound mesh `object` lets through: 1, none of it
    /// blocks (glass, a fence); 0, all of it; [`TRANSMISSION`] unless set.
    pub fn set_sound_transmission(&self, object: ObjectId, transmission: f32) {
        let mut inner = self.inner.borrow_mut();
        if let Some(sound) = inner.audio.as_mut() {
            let t = if transmission.is_nan() {
                TRANSMISSION
            } else {
                transmission.clamp(0., 1.)
            };
            sound.transmission.retain(|(id, _)| *id != object);
            sound.transmission.push((object, t));
            // Each emitter's rays again.
            for placed in sound.emitters.iter_mut().filter_map(|(_, p)| p.as_mut()) {
                placed.rayed = None;
            }
        }
        drop(inner);
        self.view.invalidate();
    }

    /// Rays a frame for occlusion, at most (8 unless set), each from the
    /// listener to an emitter whose end of it moved, taken in turn.
    pub fn set_occlusion_rays(&self, rays: usize) {
        if let Some(sound) = self.inner.borrow_mut().audio.as_mut() {
            sound.rays = rays;
        }
    }

    /// Rays cast for occlusion in the last frame.
    pub fn audio_rays(&self) -> usize {
        self.inner.borrow().audio.as_ref().map_or(0, |s| s.cast)
    }

    /// Its sound's frame at `time`: what moved since the last is sent to
    /// the engine. Its view does this after each frame it draws; call it
    /// to hear a world drawn by no view (offline, in a test).
    pub fn update_audio(&self, time: Instant) {
        frame(&self.inner, time);
    }
}

/// Puts `value` in the first free slot of `slots`: its index and
/// generation.
fn claim<T>(slots: &mut Vec<(u32, Option<T>)>, value: T) -> (u32, u32) {
    if let Some(index) = slots.iter().position(|(_, v)| v.is_none()) {
        let slot = &mut slots[index];
        slot.0 += 1;
        slot.1 = Some(value);
        (index as u32, slot.0)
    } else {
        slots.push((0, Some(value)));
        (slots.len() as u32 - 1, 0)
    }
}
