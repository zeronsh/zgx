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

/// Triangles a frame's rays may test, besides their number: rays past it
/// wait for the next frame.
const TRIANGLES: usize = 100_000;

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
    binaural: bool,
    pitch: f32,
    start: f32,
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
            binaural: false,
            pitch: 1.,
            start: 0.,
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
    /// Its own pitch (2 an octave up), times Doppler's: a crowd of the same
    /// sound, each a little apart.
    pub fn pitch(mut self, pitch: f32) -> Emitter3d {
        self.pitch = pitch;
        self
    }
    /// Starts `seconds` into its sound: a crowd of the same loop, each
    /// somewhere else in it (round the loop, however far). A one-shot
    /// started past its end ends at once.
    pub fn start(mut self, seconds: f32) -> Emitter3d {
        self.start = seconds;
        self
    }
    /// Heard through a head (HRTF: the spherical head,
    /// `zgx_audio::effects::Binaural`) rather than panned: where it is
    /// round the listener, above and behind too, for headphones. Each such
    /// emitter takes one of the world's heads, each a bus of the engine's
    /// with a binaural on it, made as needed, eight at most and kept for
    /// the world's life; one let go is given to the next. With all eight
    /// taken, it's panned.
    #[cfg(feature = "hrtf")]
    pub fn binaural(mut self, binaural: bool) -> Emitter3d {
        self.binaural = binaural;
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
    /// Its own pitch, which Doppler's multiplies.
    pitch: f32,
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
        shape.pitch * pitch,
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
    /// What it last sent each room.
    rooms: [f32; ROOMS],
    /// The head it's heard through, if binaural.
    head: Option<usize>,
}

/// Heads a world makes at most: each a bus of the engine's.
#[cfg(feature = "hrtf")]
const HEADS: usize = 8;

/// A bus with a binaural on it, heard from where its emitter is, given to
/// one emitter at a time.
struct Head {
    bus: Bus,
    binaural: EffectHandle,
    busy: bool,
    /// The azimuth and elevation last sent.
    facing: [f32; 2],
}

/// A head's parameters, as the binaural numbers them.
const AZIMUTH: usize = 0;
const ELEVATION: usize = 1;
#[cfg(feature = "hrtf")]
const _: () = assert!(
    AZIMUTH == zgx_audio::effects::Binaural::AZIMUTH
        && ELEVATION == zgx_audio::effects::Binaural::ELEVATION
);

/// Heads made so far, for each one's bus's name.
#[cfg(feature = "hrtf")]
static HEADS_MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Where `point` is round the listener's head, in degrees: its azimuth (0
/// ahead, positive right) and elevation (positive up).
fn direction(heard: &Heard, point: Vec3) -> [f32; 2] {
    let to = point - heard.position;
    let forward = heard.forward.normalized();
    let right = forward.cross(heard.up).normalized();
    let up = right.cross(forward);
    let (x, y, z) = (to.dot(right), to.dot(up), to.dot(forward));
    [
        x.atan2(z).to_degrees(),
        y.atan2((x * x + z * z).sqrt()).to_degrees(),
    ]
}

/// Reverbs a world plays its zones through at once: the listener's zone
/// and the one it's fading from.
const ROOMS: usize = 2;

/// One of a world's reverbs: a bus of its own under the world's, a reverb
/// on it heard all wet, the zone it's given to, and since when it's been
/// silent (free once its decay has passed).
struct Room {
    bus: Bus,
    reverb: EffectHandle,
    zone: Option<ReverbZoneId>,
    quiet_since: Option<Instant>,
    decay: f32,
}

/// What an emitter at `position` sends `room`, the listener at
/// `listener`: its zone's wet, times how much the listener hears of it,
/// times a half more for an emitter in it (one outside sends half).
/// Nothing to a room no zone has, or whose zone's gone quiet.
fn send_level(
    zones: &[(u32, Option<ReverbZone>)],
    room: &Room,
    listener: Vec3,
    position: Vec3,
) -> f32 {
    if room.quiet_since.is_some() {
        return 0.;
    }
    let Some(zone) = room.zone.and_then(|id| zones[id.index as usize].1) else {
        return 0.;
    };
    zone.preset.wet * zone.weight(listener) * (0.5 + 0.5 * zone.weight(position))
}

/// Rooms made so far, for each one's bus's name.
static ROOMS_MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A mesh that can block sound, as a frame's rays see it.
struct Blocker {
    index: usize,
    matrix: crate::Mat4,
    /// Its geometry's box's centre and half-sizes, and its box in the
    /// world.
    centre: Vec3,
    half: Vec3,
    min: Vec3,
    max: Vec3,
    through: f32,
    triangles: usize,
    /// Whether it's placed under another object: one an ancestor's move
    /// moves.
    parented: bool,
}

/// The box round a box of `centre` and `half` sizes placed by `matrix`:
/// its centre moved, its half-sizes summed along each world axis by how
/// much of each the matrix turns there (Arvo's), without its corners.
fn placed_box(matrix: &crate::Mat4, centre: Vec3, half: Vec3) -> (Vec3, Vec3) {
    let c = &matrix.columns;
    let middle = matrix.project(centre);
    let reach = |r: usize| c[0][r].abs() * half.x + c[1][r].abs() * half.y + c[2][r].abs() * half.z;
    let reach = vec3(reach(0), reach(1), reach(2));
    (middle - reach, middle + reach)
}

/// Object `index`'s matrix in the world, walked up its parents: as the
/// world's own (`None` hidden, or under a hidden one, or a chain past 64),
/// with nothing allocated, for the few objects a frame places.
fn walk(inner: &Inner, index: usize) -> Option<crate::Mat4> {
    let mut matrix = inner.locals.get(index).copied().flatten()?;
    let mut parent = inner.objects[index].value.as_ref()?.parent;
    for _ in 0..64 {
        let Some(id) = parent else {
            return Some(matrix);
        };
        let index = inner.live(id.index, id.generation)?;
        matrix = inner.locals.get(index).copied().flatten()? * matrix;
        parent = inner.objects[index].value.as_ref()?.parent;
    }
    None
}

/// A ray's segment, its direction's reciprocals worked out once for the
/// many boxes it's tested against.
struct Segment {
    from: Vec3,
    inverse: [f32; 3],
}

impl Segment {
    fn new(from: Vec3, to: Vec3) -> Segment {
        let along = to - from;
        Segment {
            from,
            inverse: [along.x, along.y, along.z].map(|d| 1. / d),
        }
    }

    /// Whether it passes through the box from `min` to `max` (the slabs'
    /// test, multiplying where it would divide).
    #[inline]
    fn through(&self, min: Vec3, max: Vec3) -> bool {
        let (mut enter, mut leave) = (0f32, 1f32);
        for ((f, inverse), (lo, hi)) in [self.from.x, self.from.y, self.from.z]
            .into_iter()
            .zip(self.inverse)
            .zip([(min.x, max.x), (min.y, max.y), (min.z, max.z)])
        {
            if inverse.is_infinite() {
                // Along the slab: inside it or never.
                if f < lo || f > hi {
                    return false;
                }
                continue;
            }
            let (a, b) = ((lo - f) * inverse, (hi - f) * inverse);
            enter = enter.max(a.min(b));
            leave = leave.min(a.max(b));
            if enter > leave {
                return false;
            }
        }
        true
    }
}

/// A tree of the blockers' boxes, each node's box round its children's:
/// a ray goes down only where it passes, so a still world's thousands of
/// meshes cost it a few dozen boxes. Made when the meshes have held still
/// for a frame (`Sound3d::blockers`).
#[derive(Default)]
struct Tree {
    nodes: Vec<Node>,
    /// The blockers, in the order the leaves hold them.
    order: Vec<u32>,
}

struct Node {
    min: Vec3,
    max: Vec3,
    /// A leaf: its blockers, `order[start..start + count]`. Else its
    /// children: the next node, and `start`.
    start: u32,
    count: u32,
}

/// Blockers a leaf holds at most.
const LEAF: usize = 4;

impl Tree {
    fn new(blockers: &[Blocker]) -> Tree {
        let mut tree = Tree {
            nodes: Vec::with_capacity(blockers.len() / 2 + 1),
            order: (0..blockers.len() as u32).collect(),
        };
        if !blockers.is_empty() {
            tree.split(blockers, 0, blockers.len(), 0);
        }
        tree
    }

    /// The node for `order[start..end]`, and below it.
    fn split(&mut self, blockers: &[Blocker], start: usize, end: usize, depth: u32) {
        let here = self.nodes.len();
        let (mut min, mut max) = (
            vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY),
            vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        );
        for &b in &self.order[start..end] {
            min = min.min(blockers[b as usize].min);
            max = max.max(blockers[b as usize].max);
        }
        self.nodes.push(Node {
            min,
            max,
            start: start as u32,
            count: (end - start) as u32,
        });
        if end - start <= LEAF || depth > 40 {
            return;
        }
        // Halved at the middle along its longest side, by the boxes'
        // centres.
        let size = max - min;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        let centre = |b: &u32| {
            let b = &blockers[*b as usize];
            let c = b.min + b.max;
            [c.x, c.y, c.z][axis]
        };
        let middle = (start + end) / 2;
        self.order[start..end]
            .select_nth_unstable_by(middle - start, |a, b| centre(a).total_cmp(&centre(b)));
        self.split(blockers, start, middle, depth + 1);
        let right = self.nodes.len();
        self.split(blockers, middle, end, depth + 1);
        self.nodes[here].start = right as u32;
        self.nodes[here].count = 0;
    }

    /// Each node's box made again round its blockers' (now `blockers`, the
    /// same ones in the same places in the list): children come after
    /// their parent, so from the last node back each is made from what's
    /// already made.
    fn refit(&mut self, blockers: &[Blocker]) {
        for at in (0..self.nodes.len()).rev() {
            let node = &self.nodes[at];
            let (min, max) = if node.count > 0 {
                let leaf = &self.order[node.start as usize..(node.start + node.count) as usize];
                leaf.iter().fold(
                    (
                        vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY),
                        vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
                    ),
                    |(min, max), &b| {
                        let b = &blockers[b as usize];
                        (min.min(b.min), max.max(b.max))
                    },
                )
            } else {
                let (left, right) = (&self.nodes[at + 1], &self.nodes[node.start as usize]);
                (left.min.min(right.min), left.max.max(right.max))
            };
            self.nodes[at].min = min;
            self.nodes[at].max = max;
        }
    }

    /// Each blocker whose box `segment` passes through, to `f`.
    fn each(&self, blockers: &[Blocker], segment: &Segment, mut f: impl FnMut(&Blocker)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let node = &self.nodes[at];
            if !segment.through(node.min, node.max) {
                continue;
            }
            if node.count > 0 {
                let leaf = &self.order[node.start as usize..(node.start + node.count) as usize];
                for &b in leaf {
                    let blocker = &blockers[b as usize];
                    if segment.through(blocker.min, blocker.max) {
                        f(blocker);
                    }
                }
            } else {
                stack.push(node.start as usize);
                stack.push(at + 1);
            }
        }
    }
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
    /// Its reverbs, made with its first zone.
    rooms: Vec<Room>,
    /// Its binaural heads, made as binaural emitters need them.
    heads: Vec<Head>,
    emitters: Vec<(u32, Option<Placed>)>,
    zones: Vec<(u32, Option<ReverbZone>)>,
    listener: Listener,
    heard: Option<Heard>,
    last_frame: Option<Instant>,
    /// Whether anything moved in the last frame, for a second call at its
    /// time to say the same.
    moving: bool,
    /// Objects' transmission, set by the app, and a count of its changes.
    transmission: std::collections::HashMap<ObjectId, f32>,
    transmissions: u64,
    /// The meshes that can block, and the world's meshes count and the
    /// transmissions' when they were found (`Sound3d::blockers`).
    blocking: (Option<(u64, u64)>, Vec<Blocker>),
    /// Their tree, and the counts it was made for: made once they've held
    /// still for a frame, so meshes that move every frame aren't sorted
    /// every frame (their rays look at every box instead).
    tree: (Option<(u64, u64)>, Tree),
    /// The world's `ordering` when the blockers were last found whole; by
    /// object index, its blocker's place in the list (`u32::MAX`: none);
    /// and the blockers under another object.
    ordering: u64,
    position: Vec<u32>,
    parented: Vec<u32>,
    rays: usize,
    /// The next emitter a ray is cast for.
    cursor: usize,
    /// Rays cast last frame (`World::audio_rays`).
    cast: usize,
    /// Whether the zones changed since the rooms' sends were last worked
    /// out.
    zoned: bool,
    /// The world's count of changes to its meshes when last rayed: a wall
    /// put up or moved between two ends that are still is rayed again.
    meshes: u64,
    /// Work the tests count, not time: world matrices walked, and meshes
    /// whose triangles a ray met.
    #[cfg(test)]
    work: std::cell::Cell<[u64; 2]>,
}

fn array(v: Vec3) -> [f32; 3] {
    v.to_array()
}

impl Sound3d {
    /// Where `object` is and which way it faces, if it's there to hear.
    fn place_of(inner: &Inner, object: ObjectId) -> Option<(Vec3, Vec3)> {
        let index = inner.live(object.index, object.generation)?;
        let matrix = walk(inner, index)?;
        let position = matrix.project(Vec3::ZERO);
        let ahead = (matrix.project(vec3(0., 0., -1.)) - position).normalized();
        Some((position, ahead))
    }

    /// The listener now, its velocity from where it was `step` seconds ago.
    fn hear(&self, inner: &Inner, step: f64) -> Heard {
        let (position, forward, up) = match self.listener {
            Listener::Object(id) => inner
                .live(id.index, id.generation)
                .and_then(|index| walk(inner, index))
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

    /// The meshes that can block, kept while the world's meshes and the
    /// transmissions set are as they were: each with its world matrix and
    /// its box in the world, worked out once for every ray after. Objects
    /// gone are let go of their transmissions here.
    fn blockers(&mut self, inner: &Inner) {
        let made = (inner.meshes, self.transmissions);
        if self.blocking.0 == Some(made) {
            if self.tree.0 != Some(made) {
                self.tree = (Some(made), Tree::new(&self.blocking.1));
            }
            return;
        }
        // Only moves, poses and cutoffs since (the world's `ordering` and
        // the transmissions as they were): the same meshes block, and only
        // those stamped since, or under another, are placed again.
        if let Some((meshes, transmissions)) = self.blocking.0
            && transmissions == self.transmissions
            && self.ordering == inner.ordering
        {
            let mut moved = false;
            // The objects stamped since, from the stamps (a word an object),
            // and those under others: theirs, placed again.
            let stamped = inner
                .stamps
                .iter()
                .enumerate()
                .filter(|&(_, &s)| s > meshes)
                .filter_map(|(index, _)| self.position.get(index).copied())
                .filter(|&at| at != u32::MAX);
            let again: Vec<u32> = stamped.chain(self.parented.iter().copied()).collect();
            for at in again {
                let blocker = &mut self.blocking.1[at as usize];
                #[cfg(test)]
                self.work.set({
                    let mut w = self.work.get();
                    w[0] += 1;
                    w
                });
                // Gone from sight under a hidden one: blocks nothing.
                let matrix = walk(inner, blocker.index).unwrap_or(crate::Mat4 {
                    columns: [[0.; 4]; 4],
                });
                blocker.matrix = matrix;
                (blocker.min, blocker.max) = placed_box(&matrix, blocker.centre, blocker.half);
                moved = true;
            }
            // The tree refitted round what moved, or made, if the meshes
            // haven't held still since they were found: what keeps moving
            // is sorted once, and refitted after.
            if self.tree.0.is_none() {
                self.tree.1 = Tree::new(&self.blocking.1);
            } else if moved {
                self.tree.1.refit(&self.blocking.1);
            }
            self.tree.0 = Some(made);
            self.blocking.0 = Some(made);
            return;
        }
        self.ordering = inner.ordering;
        self.transmission
            .retain(|id, _| inner.live(id.index, id.generation).is_some());
        let before = std::mem::take(&mut self.blocking.1);
        let mut blocking = Vec::with_capacity(before.len());
        for (index, slot) in inner.objects.iter().enumerate() {
            let Some(geometry) = slot.value.as_ref().and_then(|o| o.geometry.as_ref()) else {
                continue;
            };
            let id = ObjectId {
                index: index as u32,
                generation: slot.generation,
            };
            // (Most worlds set none: no hashing a lookup for each mesh.)
            let through = if self.transmission.is_empty() {
                TRANSMISSION
            } else {
                self.transmission.get(&id).copied().unwrap_or(TRANSMISSION)
            };
            if through >= 1. {
                continue;
            }
            #[cfg(test)]
            self.count(0);
            let Some(matrix) = walk(inner, index) else {
                continue;
            };
            let (low, high) = geometry.bounds();
            let (centre, half) = ((low + high) * 0.5, (high - low) * 0.5);
            let (min, max) = placed_box(&matrix, centre, half);
            let parented = slot.value.as_ref().is_some_and(|o| o.parent.is_some());
            blocking.push(Blocker {
                index,
                matrix,
                centre,
                half,
                parented,
                min,
                max,
                through,
                triangles: geometry.indices().len() / 3,
            });
        }
        // Where each object's blocker is, and which are under another.
        self.position.clear();
        self.position.resize(inner.objects.len(), u32::MAX);
        self.parented.clear();
        for (at, blocker) in blocking.iter().enumerate() {
            self.position[blocker.index] = at as u32;
            if blocker.parented {
                self.parented.push(at as u32);
            }
        }
        // The same meshes block as before, only moved: the tree keeps its
        // shape, its boxes made again round what moved. Others, it's made
        // anew once they hold still.
        let same = before.len() == blocking.len()
            && before
                .iter()
                .zip(&blocking)
                .all(|(a, b)| a.index == b.index);
        if same && self.tree.0.is_some() {
            self.tree.1.refit(&blocking);
            self.tree.0 = Some(made);
        } else {
            self.tree.0 = None;
        }
        self.blocking = (Some(made), blocking);
    }

    /// What the meshes between `from` and `to` let through: each one met
    /// multiplies the gain by its transmission and the cutoff by its
    /// transmission to the power 1.5 (a quarter: -12 dB, 2.5 kHz).
    /// `own` (the emitter's object) and `listener` (its, if it has one)
    /// don't count. Only a mesh whose box the segment passes through has
    /// its triangles looked at; how many, it adds to `triangles`.
    fn occlusion(
        &self,
        inner: &Inner,
        from: Vec3,
        to: Vec3,
        own: ObjectId,
        triangles: &mut usize,
    ) -> (f32, f32) {
        let listener = match self.listener {
            Listener::Object(id) => Some(id.index as usize),
            Listener::Camera => None,
        };
        let (mut gain, mut cutoff) = (1f32, OPEN);
        let segment = Segment::new(from, to);
        let mut met = |blocker: &Blocker| {
            if blocker.index == own.index as usize || Some(blocker.index) == listener {
                return;
            }
            let Some(geometry) = inner.objects[blocker.index]
                .value
                .as_ref()
                .and_then(|o| o.geometry.as_ref())
            else {
                return;
            };
            #[cfg(test)]
            self.count(1);
            *triangles += blocker.triangles;
            if crate::pick::meet(geometry, blocker.matrix, from, to).is_some() {
                gain *= blocker.through;
                cutoff *= blocker.through * blocker.through.sqrt();
            }
        };
        let blockers = &self.blocking.1;
        if self.tree.0.is_some() && self.tree.0 == self.blocking.0 {
            self.tree.1.each(blockers, &segment, met);
        } else {
            for blocker in blockers {
                if segment.through(blocker.min, blocker.max) {
                    met(blocker);
                }
            }
        }
        (gain, cutoff.max(100.))
    }

    #[cfg(test)]
    fn count(&self, which: usize) {
        let mut work = self.work.get();
        work[which] += 1;
        self.work.set(work);
    }

    /// A free head for a binaural emitter: one let go, or one made, eight at
    /// most. None when all are taken (or the engine has no bus to spare):
    /// it's panned.
    fn head(&mut self) -> Option<usize> {
        if let Some(free) = self.heads.iter().position(|h| !h.busy) {
            self.heads[free].busy = true;
            return Some(free);
        }
        #[cfg(feature = "hrtf")]
        if self.heads.len() < HEADS {
            let n = HEADS_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let bus = self
                .audio
                .add_bus(&format!("3d head {n}"), &self.bus)
                .ok()?;
            let binaural = bus
                .add_effect(zgx_audio::effects::Binaural::spherical())
                .ok()?;
            self.heads.push(Head {
                bus,
                binaural,
                busy: true,
                facing: [f32::NAN; 2],
            });
            return Some(self.heads.len() - 1);
        }
        None
    }

    /// Each emitter's sends to the rooms ([`send_level`]), once they're
    /// given out ([`Self::assign`]): sent only where they changed.
    fn rooms(&mut self, listener: Vec3, time: Instant, ramp: Ramp) {
        self.assign(listener, time);
        // Each emitter's send to each room.
        for placed in self.emitters.iter_mut().filter_map(|(_, p)| p.as_mut()) {
            let Some((position, _)) = placed.last else {
                continue;
            };
            for (k, room) in self.rooms.iter().enumerate() {
                let level = send_level(&self.zones, room, listener, position);
                if (level - placed.rooms[k]).abs() > 1e-4
                    && placed.voice.set_send(&room.bus, level, ramp).is_ok()
                {
                    placed.rooms[k] = level;
                }
            }
        }
    }

    /// The rooms given to the zones the listener hears (at most two), a
    /// zone keeping its room; a room no longer heard freed once its decay
    /// has passed.
    fn assign(&mut self, listener: Vec3, time: Instant) {
        if self.rooms.is_empty() {
            return;
        }
        let mut heard: Vec<(ReverbZoneId, f32)> = self
            .zones
            .iter()
            .enumerate()
            .filter_map(|(index, (generation, zone))| {
                let id = ReverbZoneId {
                    index: index as u32,
                    generation: *generation,
                };
                zone.as_ref().map(|z| (id, z.weight(listener)))
            })
            .filter(|(_, w)| *w > 0.)
            .collect();
        heard.sort_by(|a, b| b.1.total_cmp(&a.1));
        heard.truncate(ROOMS);
        // Rooms whose zone is no longer heard go quiet, and free after their
        // decay.
        for room in &mut self.rooms {
            let held = room
                .zone
                .is_some_and(|z| heard.iter().any(|(id, _)| *id == z));
            if held {
                room.quiet_since = None;
            } else if room.zone.is_some() {
                let since = *room.quiet_since.get_or_insert(time);
                if time.saturating_duration_since(since).as_secs_f32() > room.decay + 0.25 {
                    room.zone = None;
                    room.quiet_since = None;
                }
            }
        }
        // Zones heard without a room take a free one, made for them.
        for (id, _) in &heard {
            if self.rooms.iter().any(|r| r.zone == Some(*id)) {
                continue;
            }
            let Some(zone) = self.zones[id.index as usize].1 else {
                continue;
            };
            if let Some(room) = self.rooms.iter_mut().find(|r| r.zone.is_none()) {
                let p = zone.preset;
                let _ = room.reverb.set(Reverb::SIZE, p.size, Ramp::NONE);
                let _ = room.reverb.set(Reverb::DECAY, p.decay, Ramp::NONE);
                let _ = room.reverb.set(Reverb::DAMPING, p.damping, Ramp::NONE);
                room.zone = Some(*id);
                room.decay = p.decay;
            }
        }
    }

    /// One frame at `time`: sends what moved. Returns whether anything
    /// moved, so another frame must follow.
    fn frame(&mut self, inner: &Inner, time: Instant) -> bool {
        // Once a frame time: a view shown in two boxes, or drawn by two
        // renderers, runs this after each, and a second step of no time
        // would make every velocity nothing, and Doppler and pan flip each
        // frame. Whichever comes first does it.
        if self.last_frame == Some(time) {
            return self.moving;
        }
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
        // Whether any emitter moved, for the zones' sends.
        let mut shifted = false;
        // Those whose sound has ended, let go: a one-shot is sent nothing
        // after it's done.
        for (_, slot) in self.emitters.iter_mut() {
            if let Some(placed) = slot.as_ref()
                && !placed.voice.playing()
            {
                if let Some(head) = placed.head {
                    self.heads[head].busy = false;
                }
                *slot = None;
            }
        }
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
                    if let Some(head) = placed.head {
                        self.heads[head].busy = false;
                    }
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
                shifted = true;
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
        self.blockers(inner);
        self.cast = 0;
        let count = self.emitters.len();
        let start = self.cursor;
        let mut looked = 0;
        // What the rays may look at a frame besides their number: no frame
        // tests more triangles than this, once it has cast one ray.
        let mut triangles = 0;
        while looked < count && self.cast < self.rays && (self.cast == 0 || triangles < TRIANGLES) {
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
            let occluded = self.occlusion(
                inner,
                heard.position,
                position,
                placed.object,
                &mut triangles,
            );
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
            let [mut level, mut pan, pitch, air] =
                place(&placed.shape, position, ahead, placed.velocity, &listening);
            let (gain, cutoff) = placed.occluded;
            let mut faced = true;
            if let Some(head) = placed.head {
                // Centred into its head, which hears its left and right
                // averaged: at the equal-power centre's 1/√2, made up.
                level *= std::f32::consts::SQRT_2;
                pan = 0.;
                let head = &mut self.heads[head];
                let facing = direction(&heard, position);
                if head.facing != facing {
                    faced = head.binaural.set(AZIMUTH, facing[0], ramp).is_ok()
                        && head.binaural.set(ELEVATION, facing[1], ramp).is_ok();
                    if faced {
                        head.facing = facing;
                    }
                }
            }
            // Occlusion moves no faster than `SETTLE`: a wall slides in.
            let slow = Ramp::seconds(step.max(SETTLE));
            let sent = placed.voice.set_gain(level * gain, slow).is_ok()
                && placed.voice.set_pan(pan, ramp).is_ok()
                && placed.voice.set_pitch(pitch, ramp).is_ok()
                && placed.voice.set_cutoff(air.min(cutoff), slow).is_ok()
                && faced;
            // Not all of it got through (the queue was full): again next
            // frame.
            placed.stale = !sent;
        }
        // The zones' reverbs, where the listener is and each emitter is:
        // worked out when either moved, the zones changed, or a room is
        // waiting to be freed.
        let waiting = self.rooms.iter().any(|r| r.quiet_since.is_some());
        if listener_moved || shifted || self.zoned || waiting {
            self.zoned = false;
            self.rooms(heard.position, time, Ramp::seconds(step.max(SETTLE)));
        }
        self.moving = moving;
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
        // The zones' rooms given out, for its sends.
        if self.zoned {
            let now = self.last_frame.unwrap_or_else(Instant::now);
            self.assign(heard.position, now);
        }
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
            pitch: emitter.pitch,
        };
        let head = if emitter.binaural { self.head() } else { None };
        let bus = head.map_or(&self.bus, |h| &self.heads[h].bus);
        // Its start in its sound's own frames.
        let offset = (f64::from(emitter.start.max(0.)) * f64::from(emitter.sound.rate())) as u64;
        let mut play = Play::new()
            .bus(bus)
            .looping(emitter.looping)
            .offset(offset)
            .law(PanLaw::EqualPower);
        // Placed where it is now, so it starts there.
        let at = Self::place_of(inner, object);
        let mut rooms = [0.; ROOMS];
        play = match at {
            Some((position, ahead)) => {
                let [mut gain, mut pan, pitch, cutoff] =
                    place(&shape, position, ahead, Vec3::ZERO, &listener);
                if let Some(h) = head {
                    // Its head faces it before its first frame.
                    gain *= std::f32::consts::SQRT_2;
                    pan = 0.;
                    let head = &mut self.heads[h];
                    let facing = direction(&heard, position);
                    if head.binaural.set(AZIMUTH, facing[0], Ramp::NONE).is_ok()
                        && head.binaural.set(ELEVATION, facing[1], Ramp::NONE).is_ok()
                    {
                        head.facing = facing;
                    }
                }
                // Its sends to the zones' reverbs from the start too: a sound
                // that starts with a click is heard in its room.
                for (k, room) in self.rooms.iter().enumerate() {
                    rooms[k] = send_level(&self.zones, room, heard.position, position);
                    if rooms[k] > 0. {
                        play = play.send(&room.bus, rooms[k]);
                    }
                }
                play.gain(gain).pan(pan).pitch(pitch).cutoff(cutoff)
            }
            None => play.gain(0.).pitch(shape.pitch),
        };
        let voice = match self.audio.play(&emitter.sound, play) {
            Ok(voice) => voice,
            Err(error) => {
                if let Some(h) = head {
                    self.heads[h].busy = false;
                }
                return Err(error);
            }
        };
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
            rooms,
            head,
        };
        let (index, generation) = claim(&mut self.emitters, placed);
        Ok(Emitter3dId { index, generation })
    }
}

impl Drop for Sound3d {
    /// A world dropped, or given another engine: its sounds stop, and its
    /// reverb comes off its bus.
    fn drop(&mut self) {
        for placed in self.emitters.drain(..).filter_map(|(_, p)| p) {
            let _ = placed.voice.stop();
        }
        for room in self.rooms.drain(..) {
            let _ = room.reverb.remove();
        }
        for head in self.heads.drain(..) {
            let _ = head.binaural.remove();
        }
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
    /// Gives it sound through `audio`: its emitters play on `bus`, which
    /// its reverb zones' reverb goes on. The engine's `world` bus
    /// (`audio.bus("world")`) is the one meant for it, and worlds can share
    /// it: an engine has few buses, and none can be taken away. Again
    /// starts over: what it played before is stopped.
    pub fn set_audio(&self, audio: &Audio, bus: &Bus) -> Result<(), Error> {
        let bus = bus.clone();
        let mut inner = self.inner.borrow_mut();
        // The one before, if any, dropped: its sounds stop.
        drop(inner.audio.take());
        inner.audio = Some(Box::new(Sound3d {
            audio: audio.clone(),
            bus,
            rooms: Vec::new(),
            heads: Vec::new(),
            emitters: Vec::new(),
            zones: Vec::new(),
            listener: Listener::Camera,
            heard: None,
            last_frame: None,
            moving: false,
            transmission: Default::default(),
            transmissions: 0,
            blocking: (None, Vec::new()),
            tree: (None, Tree::default()),
            ordering: 0,
            position: Vec::new(),
            parented: Vec::new(),
            rays: RAYS,
            cursor: 0,
            cast: 0,
            zoned: true,
            meshes: 0,
            #[cfg(test)]
            work: Default::default(),
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
            if let Some(head) = placed.head {
                sound.heads[head].busy = false;
            }
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
    /// its reverb, each emitter sending to it (one outside it at half).
    /// Two zones are heard at once at most: the listener's and the one
    /// it's fading from, each through a reverb of its own, given to it
    /// while it's heard and to another zone once its decay has passed.
    /// The first zone makes those two reverbs, each on a bus of its own
    /// under the world's: two of the engine's buses, for the world's life.
    pub fn add_reverb_zone(&self, zone: ReverbZone) -> Result<ReverbZoneId, Error> {
        let mut inner = self.inner.borrow_mut();
        let Some(sound) = inner.audio.as_mut() else {
            return Err(Error::Effect("no engine: World::set_audio first".into()));
        };
        if sound.rooms.is_empty() {
            for _ in 0..ROOMS {
                let n = ROOMS_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let bus = sound.audio.add_bus(&format!("3d reverb {n}"), &sound.bus)?;
                let p = zone.preset;
                let reverb =
                    bus.add_effect(Reverb::new(p.size, p.decay).damping(p.damping).mix(1., 0.))?;
                sound.rooms.push(Room {
                    bus,
                    reverb,
                    zone: None,
                    quiet_since: None,
                    decay: p.decay,
                });
            }
        }
        let (index, generation) = claim(&mut sound.zones, zone);
        sound.zoned = true;
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
            sound.zoned = true;
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
            sound.transmission.insert(object, t);
            sound.transmissions += 1;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Geometry, Material, Object, Transform};

    fn work(world: &World) -> [u64; 2] {
        world.inner.borrow().audio.as_ref().unwrap().work.get()
    }

    #[test]
    fn a_moving_listener_s_rays_cost_what_they_meet_not_the_world() {
        // 2,000 meshes, none between the listener and its 8 emitters: a
        // frame where only the listener moves walks no mesh's matrix and
        // tests no mesh's triangles.
        let (audio, _mixer) = zgx_audio::Audio::offline(zgx_audio::Config::default()).unwrap();
        let world = World::new();
        world.set_camera(Camera::looking_at([0., 0., 0.], [0., 0., -1.]));
        world
            .set_audio(&audio, &audio.bus("world").unwrap())
            .unwrap();
        for i in 0..2_000 {
            let id = world.add(Object::mesh(
                Geometry::cuboid(0.5, 0.5, 0.5),
                Material::default(),
            ));
            world.set_transform(
                id,
                Transform::at([(i % 50) as f32 * 2. - 50., 20. + (i / 50) as f32, -10.]),
            );
        }
        let tone = zgx_audio::Sound::from_samples(&[0., 0.5], 1, 48_000).unwrap();
        for i in 0..8 {
            let id = world.add(Object::group());
            world.set_transform(id, Transform::at([i as f32 - 4., 0., -6.]));
            world
                .add_emitter3d(id, Emitter3d::new(&tone).looping(true))
                .unwrap();
        }
        let start = Instant::now();
        world.update_audio(start);
        for k in 1..4 {
            world.set_camera(Camera::looking_at([0.1 * k as f32, 0., 0.], [0., 0., -1.]));
            let before = work(&world);
            world.update_audio(start + std::time::Duration::from_millis(10 * k));
            let after = work(&world);
            assert_eq!(world.audio_rays(), 8);
            let walked = after[0] - before[0];
            let met = after[1] - before[1];
            assert!(walked <= 64, "frame {k}: {walked} matrices walked");
            assert_eq!(met, 0, "frame {k}: {met} meshes' triangles tested");
        }
    }

    #[test]
    fn a_left_zone_keeps_its_reverb_until_its_decay_has_passed() {
        // Three zones far apart; the listener steps from the room to the
        // hall to the warehouse. Both reverbs are still ringing when it
        // reaches the warehouse, which waits; once the room's 0.6 s decay
        // has passed, the warehouse takes the room's reverb, the hall's
        // still held.
        let (audio, _mixer) = zgx_audio::Audio::offline(zgx_audio::Config::default()).unwrap();
        let world = World::new();
        world
            .set_audio(&audio, &audio.bus("world").unwrap())
            .unwrap();
        let zone = |x: f32, preset| {
            world
                .add_reverb_zone(
                    ReverbZone::box_([x - 5., -5., -5.], [x + 5., 5., 5.])
                        .preset(preset)
                        .fade(1.),
                )
                .unwrap()
        };
        let room = zone(-30., ReverbPreset::ROOM);
        let hall = zone(0., ReverbPreset::HALL);
        let warehouse = zone(30., ReverbPreset::WAREHOUSE);
        let held = |world: &World| -> Vec<Option<ReverbZoneId>> {
            let inner = world.inner.borrow();
            inner
                .audio
                .as_ref()
                .unwrap()
                .rooms
                .iter()
                .map(|r| r.zone)
                .collect()
        };
        let start = Instant::now();
        let at = |ms: u64, x: f32| {
            world.set_camera(Camera::looking_at([x, 0., 0.], [x, 0., -1.]));
            world.update_audio(start + std::time::Duration::from_millis(ms));
        };
        at(0, -30.);
        at(100, 0.);
        at(200, 30.);
        assert_eq!(held(&world), [Some(room), Some(hall)]);
        at(800, 30.);
        assert_eq!(
            held(&world),
            [Some(room), Some(hall)],
            "freed before its decay"
        );
        at(1_200, 30.);
        assert_eq!(held(&world), [Some(warehouse), Some(hall)]);
    }

    #[cfg(feature = "hrtf")]
    #[test]
    fn binaural_emitters_share_eight_heads_and_pan_past_them() {
        let (audio, _mixer) = zgx_audio::Audio::offline(zgx_audio::Config::default()).unwrap();
        let world = World::new();
        world
            .set_audio(&audio, &audio.bus("world").unwrap())
            .unwrap();
        let tone = zgx_audio::Sound::from_samples(&[0., 0.5], 1, 48_000).unwrap();
        let at = world.add(Object::group());
        let add = || {
            world
                .add_emitter3d(at, Emitter3d::new(&tone).looping(true).binaural(true))
                .unwrap()
        };
        let heads = |world: &World| -> Vec<Option<usize>> {
            let inner = world.inner.borrow();
            let sound = inner.audio.as_ref().unwrap();
            assert!(sound.heads.len() <= HEADS);
            sound
                .emitters
                .iter()
                .filter_map(|(_, p)| p.as_ref().map(|p| p.head))
                .collect()
        };
        let ids: Vec<_> = (0..9).map(|_| add()).collect();
        let given = heads(&world);
        assert_eq!(&given[..8], (0..8).map(Some).collect::<Vec<_>>());
        assert_eq!(given[8], None, "the ninth is panned");
        // One let go: the next binaural emitter takes its head, no new one.
        world.remove_emitter3d(ids[3]);
        add();
        assert!(heads(&world).contains(&Some(3)));
        assert_eq!(world.inner.borrow().audio.as_ref().unwrap().heads.len(), 8);
    }
}
