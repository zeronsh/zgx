//! A glTF model's animations (feature `gltf`): clips whose channels move
//! its nodes (`Model::clips`), posed on a model placed in a world
//! (`World::place_model`, `Placed::pose`) at any time, as often as wanted.
use crate::{ObjectId, Quat, Transform, Vec3, World, vec3};

/// One of a model's animations: its channels move nodes over `duration`
/// seconds. Clones share it.
#[derive(Clone, Debug)]
pub struct Clip(pub(crate) std::sync::Arc<ClipData>);
#[derive(Clone, Debug)]
pub(crate) struct ClipData {
    pub(crate) name: Option<String>,
    pub(crate) duration: f32,
    /// Its channels moving nodes' places (translation, rotation, scale).
    pub(crate) channels: Vec<Channel>,
    /// Its channels moving morph weights.
    pub(crate) morphs: Vec<Channel>,
    /// Its channels' key times, each list once (most clips share one
    /// among all their channels): a time is found among each once a frame.
    pub(crate) tracks: Vec<Vec<f32>>,
    /// The nodes its channels move, each once, rising.
    pub(crate) nodes: Vec<u32>,
    /// Its events (`Clip::with_event`), by time.
    pub(crate) events: Vec<(f32, std::sync::Arc<str>)>,
}
impl Clip {
    /// `channels`, their times' lists shared where equal.
    pub(crate) fn new(name: Option<String>, channels: Vec<(Vec<f32>, Channel)>) -> Self {
        let mut tracks: Vec<Vec<f32>> = Vec::new();
        let mut found = std::collections::HashMap::<Vec<u32>, usize>::new();
        let channels: Vec<Channel> = channels
            .into_iter()
            .map(|(times, mut channel)| {
                let bits = times.iter().map(|t| t.to_bits()).collect();
                channel.track = *found.entry(bits).or_insert_with(|| {
                    tracks.push(times);
                    tracks.len() - 1
                });
                channel
            })
            .collect();
        let duration = tracks
            .iter()
            .filter_map(|times| times.last().copied())
            .fold(0., f32::max);
        let mut nodes: Vec<u32> = channels.iter().map(|c| c.node as u32).collect();
        nodes.sort_unstable();
        nodes.dedup();
        // Morph weights apart: a pose's loops over places never ask.
        let (morphs, channels): (Vec<Channel>, Vec<Channel>) = channels
            .into_iter()
            .partition(|c| matches!(c.path, Path::Weight(_)));
        Self(std::sync::Arc::new(ClipData {
            name,
            duration,
            channels,
            morphs,
            tracks,
            nodes,
            events: Vec::new(),
        }))
    }
    /// A clip to make in code, named `name`: add channels, then
    /// `build`.
    pub fn builder(name: &str) -> ClipBuilder {
        ClipBuilder {
            name: Some(name.to_owned()),
            channels: Vec::new(),
        }
    }
    /// Its name in the file, if it has one.
    pub fn name(&self) -> Option<&str> {
        self.0.name.as_deref()
    }
    /// How long it runs, in seconds: its last key's time.
    pub fn duration(&self) -> f32 {
        self.0.duration
    }
    /// It with an event named `name` at `seconds` into it: an animator
    /// playing it reports the event each time its time passes there
    /// (`World::animator_update`). An event at its end is passed as it
    /// ends or loops; one at 0 as it starts and at each loop.
    pub fn with_event(mut self, seconds: f32, name: &str) -> Self {
        if !seconds.is_finite() {
            return self;
        }
        let events = &mut std::sync::Arc::make_mut(&mut self.0).events;
        let at = events.partition_point(|(t, _)| *t <= seconds);
        events.insert(at, (seconds, name.into()));
        self
    }
    /// Whether `self` and `other` are the same clip (clones of one).
    pub(crate) fn same(&self, other: &Clip) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The most keys a clip built in code holds, its channels' together (and
/// a cubic key's tangents each a key): past it, `ClipBuilder::build` fails.
pub const MAX_CLIP_KEYS: usize = 1 << 22;

/// A channel's keys for a clip built in code (`Clip::builder`): its times
/// in seconds, rising, and a value each, of `T`: `[f32; 3]` for a place or
/// a scale, a `Quat` for a turn, an `f32` for a morph weight.
#[derive(Clone, Debug, PartialEq)]
pub struct Keys<T> {
    interpolation: Interpolation,
    times: Vec<f32>,
    /// A value a key (three for cubic: in-tangent, value, out-tangent).
    values: Vec<T>,
}
impl<T> Keys<T> {
    /// Each key's value held until the next.
    pub fn step(times: Vec<f32>, values: Vec<T>) -> Self {
        Self {
            interpolation: Interpolation::Step,
            times,
            values,
        }
    }
    /// Straight between keys (turns along the shorter arc).
    pub fn linear(times: Vec<f32>, values: Vec<T>) -> Self {
        Self {
            interpolation: Interpolation::Linear,
            times,
            values,
        }
    }
    /// Hermite curves through the keys: for each, its in-tangent, its value
    /// and its out-tangent (glTF's `CUBICSPLINE`).
    pub fn cubic(times: Vec<f32>, values: Vec<[T; 3]>) -> Self {
        Self {
            interpolation: Interpolation::Cubic,
            times,
            values: values.into_iter().flatten().collect(),
        }
    }
}

/// Why a clip built in code was refused (`ClipBuilder::build`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    /// A channel with no keys.
    NoKeys,
    /// A channel's times or values not all numbers.
    NotANumber,
    /// A channel's times falling somewhere.
    NotRising,
    /// A channel with a different number of values from its keys (three
    /// a key, cubic).
    Values,
    /// More keys than `MAX_CLIP_KEYS`.
    TooMany,
}
impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoKeys => "a clip's channel has no keys",
            Self::NotANumber => "a clip's channel has a time or a value that isn't a number",
            Self::NotRising => "a clip's channel's times fall",
            Self::Values => "a clip's channel has a value a key (three a key, cubic)",
            Self::TooMany => "a clip has more keys than MAX_CLIP_KEYS",
        })
    }
}
impl std::error::Error for ClipError {}

/// A clip made in code (`Clip::builder`): channels moving nodes, by their
/// index in what it'll play on (`Placed::new`, or a model's file order).
#[derive(Clone, Debug, Default)]
pub struct ClipBuilder {
    name: Option<String>,
    channels: Vec<Built>,
}
/// A channel as built: its node, what it moves, and its keys.
#[derive(Clone, Debug)]
struct Built {
    node: usize,
    path: Path,
    keys: Keys<[f32; 4]>,
}
impl ClipBuilder {
    fn channel(mut self, node: usize, path: Path, keys: Keys<[f32; 4]>) -> Self {
        self.channels.push(Built { node, path, keys });
        self
    }
    /// Node `node`'s place in its parent's frame.
    pub fn translation(self, node: usize, keys: Keys<[f32; 3]>) -> Self {
        let keys = Keys {
            interpolation: keys.interpolation,
            times: keys.times,
            values: keys
                .values
                .into_iter()
                .map(|[x, y, z]| [x, y, z, 0.])
                .collect(),
        };
        self.channel(node, Path::Translation, keys)
    }
    /// Node `node`'s turn.
    pub fn rotation(self, node: usize, keys: Keys<Quat>) -> Self {
        let keys = Keys {
            interpolation: keys.interpolation,
            times: keys.times,
            values: keys
                .values
                .into_iter()
                .map(|q| [q.x, q.y, q.z, q.w])
                .collect(),
        };
        self.channel(node, Path::Rotation, keys)
    }
    /// Node `node`'s scale.
    pub fn scale(self, node: usize, keys: Keys<[f32; 3]>) -> Self {
        let keys = Keys {
            interpolation: keys.interpolation,
            times: keys.times,
            values: keys
                .values
                .into_iter()
                .map(|[x, y, z]| [x, y, z, 0.])
                .collect(),
        };
        self.channel(node, Path::Scale, keys)
    }
    /// Morph target `target`'s weight of node `node`'s meshes.
    pub fn weight(self, node: usize, target: usize, keys: Keys<f32>) -> Self {
        let keys = Keys {
            interpolation: keys.interpolation,
            times: keys.times,
            values: keys.values.into_iter().map(|w| [w, 0., 0., 0.]).collect(),
        };
        self.channel(node, Path::Weight(target), keys)
    }
    /// The clip, if every channel has keys, its times numbers and rising
    /// and a value a key (three for cubic), all numbers, at most
    /// `MAX_CLIP_KEYS` in all.
    pub fn build(self) -> Result<Clip, ClipError> {
        self.build_within(MAX_CLIP_KEYS)
    }
    /// As `build`, at most `limit` keys.
    pub(crate) fn build_within(self, limit: usize) -> Result<Clip, ClipError> {
        let mut total = 0usize;
        let mut channels = Vec::with_capacity(self.channels.len());
        for Built { node, path, keys } in self.channels {
            let Keys {
                interpolation,
                times,
                values,
            } = keys;
            let per = if interpolation == Interpolation::Cubic {
                3
            } else {
                1
            };
            total = total
                .saturating_add(values.len())
                .saturating_add(times.len());
            if total > limit {
                return Err(ClipError::TooMany);
            }
            if times.is_empty() {
                return Err(ClipError::NoKeys);
            }
            if !times.iter().all(|t| t.is_finite())
                || !values.iter().flatten().all(|v| v.is_finite())
            {
                return Err(ClipError::NotANumber);
            }
            if !times.windows(2).all(|t| t[0] <= t[1]) {
                return Err(ClipError::NotRising);
            }
            if values.len() != times.len() * per {
                return Err(ClipError::Values);
            }
            let channel = Channel {
                node,
                path,
                interpolation,
                track: 0,
                values,
            };
            channels.push((times, channel));
        }
        Ok(Clip::new(self.name, channels))
    }
}

/// What a channel moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Path {
    Translation,
    Rotation,
    Scale,
    /// A morph target's weight (a channel a target: `sample`'s first value).
    Weight(usize),
}
/// How a channel goes between its keys (glTF's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Interpolation {
    /// Each key's value until the next.
    Step,
    /// Straight between keys (rotations along the shortest arc).
    Linear,
    /// Hermite curves through the keys, their tangents given: three values
    /// a key (in-tangent, value, out-tangent).
    Cubic,
}
/// A node's translation, rotation or scale over time.
#[derive(Clone, Debug)]
pub(crate) struct Channel {
    pub(crate) node: usize,
    pub(crate) path: Path,
    pub(crate) interpolation: Interpolation,
    /// Its keys' times: its clip's `tracks[track]`.
    pub(crate) track: usize,
    /// Values, a key's (three for `Cubic`), xyz or xyzw.
    pub(crate) values: Vec<[f32; 4]>,
}
/// Where a time falls among a track's keys: the key at or before it, the
/// next (the same at either end, where the time is held), how far between
/// them (0 to 1) and the seconds between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Between {
    pub(crate) key: u32,
    pub(crate) next: u32,
    pub(crate) s: f32,
    pub(crate) span: f32,
}
impl Between {
    /// Where `time` falls among `times` (rising, at least one).
    pub(crate) fn find(times: &[f32], time: f32) -> Self {
        Self::from(times, time, 0)
    }
    /// As `find`, looking on from `hint`'s key first: a time a little
    /// after the last one asked (a clip playing) is a step or two on.
    #[inline]
    pub(crate) fn after(times: &[f32], time: f32, hint: Self) -> Self {
        Self::from(times, time, hint.key as usize)
    }
    #[inline]
    fn from(times: &[f32], time: f32, hint: usize) -> Self {
        let last = times.len() - 1;
        let held = |key: usize| Self {
            key: key as u32,
            next: key as u32,
            s: 0.,
            span: 0.,
        };
        // Before its first key, or no time (NaN): its first value.
        if time.partial_cmp(&times[0]) != Some(std::cmp::Ordering::Greater) {
            return held(0);
        }
        if time >= times[last] {
            return held(last);
        }
        // The last key at or before `time` (times[last] is after it).
        let mut key = hint;
        if key >= last || times[key] > time || times[(key + 4).min(last)] <= time {
            key = times.partition_point(|&t| t <= time) - 1;
        } else {
            while times[key + 1] <= time {
                key += 1;
            }
        }
        let (t0, t1) = (times[key], times[key + 1]);
        let span = t1 - t0;
        Self {
            key: key as u32,
            next: key as u32 + 1,
            s: if span > 0. { (time - t0) / span } else { 0. },
            span,
        }
    }
}
impl Channel {
    /// Its value at `time` (seconds) among `times` (its track's), held
    /// before its first key and after its last.
    pub(crate) fn sample(&self, times: &[f32], time: f32) -> [f32; 4] {
        self.at(Between::find(times, time))
    }
    /// Its value at `between`.
    #[inline]
    pub(crate) fn at(&self, between: Between) -> [f32; 4] {
        let cubic = self.interpolation == Interpolation::Cubic;
        let per = if cubic { 3 } else { 1 };
        let value = |key: u32| self.values[key as usize * per + per / 2];
        let (key, next, s) = (between.key, between.next, between.s);
        if key == next {
            return value(key);
        }
        let rotation = self.path == Path::Rotation;
        match self.interpolation {
            Interpolation::Step => value(key),
            Interpolation::Linear if rotation => slerp(value(key), value(next), s),
            Interpolation::Linear => lerp(value(key), value(next), s),
            Interpolation::Cubic => {
                let span = between.span;
                let out_tangent = self.values[key as usize * 3 + 2];
                let in_tangent = self.values[next as usize * 3];
                let (s2, s3) = (s * s, s * s * s);
                let (h00, h10) = (2. * s3 - 3. * s2 + 1., s3 - 2. * s2 + s);
                let (h01, h11) = (-2. * s3 + 3. * s2, s3 - s2);
                let (p0, p1) = (value(key), value(next));
                let mut out = [0.; 4];
                for i in 0..4 {
                    out[i] = h00 * p0[i]
                        + h10 * span * out_tangent[i]
                        + h01 * p1[i]
                        + h11 * span * in_tangent[i];
                }
                if rotation { normalized(out) } else { out }
            }
        }
    }
    /// Its first key's value: where an additive layer measures it from.
    #[inline]
    pub(crate) fn first(&self) -> [f32; 4] {
        let per = if self.interpolation == Interpolation::Cubic {
            3
        } else {
            1
        };
        self.values[per / 2]
    }
}
pub(crate) fn lerp(a: [f32; 4], b: [f32; 4], s: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * s)
}
pub(crate) fn normalized(q: [f32; 4]) -> [f32; 4] {
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length > 1e-12 {
        q.map(|v| v / length)
    } else {
        [0., 0., 0., 1.]
    }
}
/// The rotation `s` of the way from `a` to `b`, along the shorter arc.
pub(crate) fn slerp(a: [f32; 4], b: [f32; 4], s: f32) -> [f32; 4] {
    let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let b = if dot < 0. {
        dot = -dot;
        b.map(|v| -v)
    } else {
        b
    };
    if dot > 0.9995 {
        return normalized(lerp(a, b, s));
    }
    let angle = dot.clamp(-1., 1.).acos();
    let (wa, wb) = (
        ((1. - s) * angle).sin() / angle.sin(),
        (s * angle).sin() / angle.sin(),
    );
    std::array::from_fn(|i| a[i] * wa + b[i] * wb)
}

/// A model placed in a world (`World::place_model`): its root, and the
/// object each of its nodes became, to pose its clips on.
#[derive(Clone, Debug)]
pub struct Placed {
    /// The group holding it: place, turn and scale it as any object.
    pub root: ObjectId,
    /// By node index; `None` for a node its scene doesn't show.
    pub(crate) nodes: Vec<Option<ObjectId>>,
    /// Each node's meshes, by node index.
    pub(crate) meshes: Vec<Vec<ObjectId>>,
    /// Its lights (`KHR_lights_punctual`), each following its node.
    pub(crate) lights: Vec<crate::LightId>,
    /// Its cameras, with the nodes holding them, in the order placed.
    pub(crate) cameras: Vec<(ObjectId, crate::gltf::ModelCamera)>,
    /// Its nodes' names in the file, by node index (none for objects made
    /// in code).
    pub(crate) names: Vec<Option<String>>,
}
impl Placed {
    /// Objects made in code, to animate as a model's nodes: `root` the
    /// group holding them, `nodes` by index (a clip built for them names
    /// these indices: `Clip::builder`).
    pub fn new(root: ObjectId, nodes: Vec<ObjectId>) -> Self {
        Self {
            root,
            meshes: vec![Vec::new(); nodes.len()],
            nodes: nodes.into_iter().map(Some).collect(),
            lights: Vec::new(),
            cameras: Vec::new(),
            names: Vec::new(),
        }
    }
    /// The object its node named `name` in the file became (the first so
    /// named, in the file's order): `None` if none is, or its scene doesn't
    /// show it.
    pub fn node_named(&self, name: &str) -> Option<ObjectId> {
        let index = self.names.iter().position(|n| n.as_deref() == Some(name))?;
        self.node(index)
    }
    /// It with `meshes` taking node `node`'s morph weights (a clip's
    /// `Keys` for its weights). Nothing for a node it hasn't.
    pub fn with_meshes(mut self, node: usize, meshes: Vec<ObjectId>) -> Self {
        if let Some(slot) = self.meshes.get_mut(node) {
            *slot = meshes;
        }
        self
    }
    /// Its lights, each following its node: set, turn off or remove them as
    /// any (`World::set_light`, `World::remove_light`).
    pub fn lights(&self) -> &[crate::LightId] {
        &self.lights
    }
    /// The object its node `index` (in its file's order) became: to mask
    /// an animator's layer by (`Mask`), or move by hand. `None` for a node
    /// its scene doesn't show.
    pub fn node(&self, index: usize) -> Option<ObjectId> {
        self.nodes.get(index).copied().flatten()
    }
    /// Its cameras where its nodes are now, in its scene's order, for
    /// `World::set_camera` (one under a hidden node left out). A camera
    /// looks along its node's −z, without its roll.
    pub fn cameras(&self, world: &World) -> Vec<crate::Camera> {
        let placed = |(node, camera): &(ObjectId, crate::gltf::ModelCamera)| {
            Some(camera.placed(&world.world_matrix(*node)?))
        };
        self.cameras.iter().filter_map(placed).collect()
    }
    /// Pose `clip` at `seconds` into it (held at its ends: loop by taking
    /// the time modulo its `duration`): the nodes its channels move take
    /// their values then. A frame is drawn only if something moved.
    pub fn pose(&self, world: &World, clip: &Clip, seconds: f32) {
        for channel in clip.0.channels.iter().chain(&clip.0.morphs) {
            let Some(Some(id)) = self.nodes.get(channel.node) else {
                continue;
            };
            let [x, y, z, w] = channel.sample(&clip.0.tracks[channel.track], seconds);
            if let Path::Weight(target) = channel.path {
                // Its meshes share the node's weights.
                for &mesh in self.meshes.get(channel.node).into_iter().flatten() {
                    world.set_morph_weight(mesh, target, x);
                }
                continue;
            }
            world.update(*id, |object| {
                let transform: &mut Transform = &mut object.transform;
                match channel.path {
                    Path::Translation => transform.position = vec3(x, y, z),
                    Path::Rotation => transform.rotation = Quat { x, y, z, w },
                    Path::Scale => transform.scale = Vec3::from([x, y, z]),
                    Path::Weight(_) => {}
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_built_in_code_refuses_what_it_cannot_play() {
        let linear = |times: Vec<f32>, values: Vec<[f32; 3]>| {
            Clip::builder("c")
                .translation(0, Keys::linear(times, values))
                .build()
        };
        assert_eq!(linear(vec![], vec![]).err(), Some(ClipError::NoKeys));
        assert_eq!(
            linear(vec![f32::NAN], vec![[0.; 3]]).err(),
            Some(ClipError::NotANumber)
        );
        assert_eq!(
            linear(vec![0.], vec![[f32::INFINITY, 0., 0.]]).err(),
            Some(ClipError::NotANumber)
        );
        assert_eq!(
            linear(vec![1., 0.], vec![[0.; 3]; 2]).err(),
            Some(ClipError::NotRising)
        );
        assert_eq!(
            linear(vec![0., 1.], vec![[0.; 3]]).err(),
            Some(ClipError::Values)
        );
        let cubic = Clip::builder("c")
            .scale(0, Keys::cubic(vec![0., 1.], vec![[[1.; 3]; 3]]))
            .build();
        assert_eq!(
            cubic.err(),
            Some(ClipError::Values),
            "three a key, two keys"
        );
        let many = Clip::builder("c")
            .weight(0, 0, Keys::step(vec![0.; 4], vec![0.; 4]))
            .weight(0, 1, Keys::step(vec![0.; 4], vec![0.; 4]))
            .build_within(12);
        assert_eq!(many.err(), Some(ClipError::TooMany));
        assert!(
            linear(vec![0., 0., 1.], vec![[0.; 3]; 3]).is_ok(),
            "times may stay"
        );
    }

    /// Keys at 1 and 3 s.
    struct Keyed(Channel);
    impl Keyed {
        fn sample(&self, time: f32) -> [f32; 4] {
            self.0.sample(&[1., 3.], time)
        }
    }
    fn channel(interpolation: Interpolation, path: Path, values: Vec<[f32; 4]>) -> Keyed {
        Keyed(Channel {
            node: 0,
            path,
            interpolation,
            track: 0,
            values,
        })
    }

    #[test]
    fn keys_are_held_at_the_ends_and_joined_between() {
        let linear = channel(
            Interpolation::Linear,
            Path::Translation,
            vec![[0., 0., 0., 0.], [4., 2., 0., 0.]],
        );
        assert_eq!(linear.sample(0.), [0., 0., 0., 0.]);
        assert_eq!(linear.sample(2.), [2., 1., 0., 0.]);
        assert_eq!(linear.sample(9.), [4., 2., 0., 0.]);
        let step = channel(
            Interpolation::Step,
            Path::Translation,
            vec![[0., 0., 0., 0.], [4., 2., 0., 0.]],
        );
        assert_eq!(step.sample(2.9), [0., 0., 0., 0.]);
        assert_eq!(step.sample(3.), [4., 2., 0., 0.]);
    }

    #[test]
    fn rotations_turn_the_short_way_at_an_even_pace() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        // None to a quarter turn about y, its second key written negated
        // (the same rotation, the long way round if taken literally).
        let rotation = channel(
            Interpolation::Linear,
            Path::Rotation,
            vec![[0., 0., 0., 1.], [0., -half, 0., -half]],
        );
        let [x, y, z, w] = rotation.sample(2.);
        let turned = Quat { x, y, z, w }.rotate(vec3(1., 0., 0.));
        // An eighth of a turn about y, the short way.
        let eighth = std::f32::consts::FRAC_PI_4;
        assert!(
            (turned - vec3(eighth.cos(), 0., -eighth.sin())).length() < 1e-4,
            "{turned:?}"
        );
    }

    #[test]
    fn a_cubic_channel_passes_through_its_keys_along_its_tangents() {
        // Keys 0 and 2, tangents 0: an ease in and out between them.
        let zero = [0.; 4];
        let cubic = channel(
            Interpolation::Cubic,
            Path::Translation,
            vec![zero, zero, zero, zero, [2., 0., 0., 0.], zero],
        );
        assert_eq!(cubic.sample(1.)[0], 0.);
        assert_eq!(cubic.sample(3.)[0], 2.);
        assert!(
            (cubic.sample(2.)[0] - 1.).abs() < 1e-6,
            "halfway at the middle"
        );
        assert!(
            cubic.sample(1.5)[0] < 0.5,
            "slower out of its first key than a straight line"
        );
    }
}
