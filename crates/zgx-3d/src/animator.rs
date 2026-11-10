//! Animators (feature `gltf`, `World::add_animator`): a placed model's clips
//! played by the world's frames, in layers. A layer has a weight, a mask
//! saying how much it moves each node, and a blend: over the layers under
//! it (`Layer::new`) or added to them as the clip moves from its first
//! key (`Layer::additive`). Within a layer a clip crossfades from the ones
//! before it. A frame samples each clip once, its key times found from
//! where the last frame left them, and writes only the nodes whose places
//! changed.
use crate::{
    ObjectId, Quat, Transform, Vec3,
    animation::{Between, Channel, Clip, Path, slerp},
    vec3,
};
use std::sync::Arc;

/// How a layer's pose meets the layers under it (`Layer`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Blend {
    /// It takes the place of theirs, by its weight: halfway at 0.5.
    #[default]
    Override,
    /// It adds to theirs how far its clips have moved from their first
    /// keys, by its weight (a breath, a flinch, a lean over a walk).
    Additive,
}

/// Which of a model's nodes a layer moves, and how much of its pose each
/// takes (0 to 1): an upper body's wave over a walk. Nodes it doesn't name
/// take none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mask {
    /// Nodes, how much, and whether those under each too, in order (a
    /// later entry wins).
    entries: Vec<(ObjectId, f32, bool)>,
}
impl Mask {
    /// No node.
    pub fn new() -> Self {
        Self::default()
    }
    /// It with `node` taking `weight` of the layer (0 to 1).
    pub fn node(mut self, node: ObjectId, weight: f32) -> Self {
        self.entries.push((node, weight.clamp(0., 1.), false));
        self
    }
    /// It with `node` and every node under it taking `weight` of the layer
    /// (0 to 1).
    pub fn below(mut self, node: ObjectId, weight: f32) -> Self {
        self.entries.push((node, weight.clamp(0., 1.), true));
        self
    }
    /// How much of a layer each of `nodes` takes, by index: `parent` says
    /// which of them each sits under.
    pub(crate) fn resolve(
        &self,
        nodes: &[Option<ObjectId>],
        parent: impl Fn(usize) -> Option<usize>,
    ) -> Vec<f32> {
        let mut weights = vec![0.; nodes.len()];
        for &(id, weight, below) in &self.entries {
            for (index, taken) in weights.iter_mut().enumerate() {
                let mut at = Some(index);
                let mut depth = 0;
                while let Some(i) = at {
                    if nodes[i] == Some(id) {
                        *taken = weight;
                        break;
                    }
                    if !below || depth > 64 {
                        break;
                    }
                    at = parent(i);
                    depth += 1;
                }
            }
        }
        weights
    }
}

/// One of an animator's layers (`World::set_layer`), the first under the
/// rest: how much of its clips' pose shows, which nodes it moves, and how
/// it meets those under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub(crate) weight: f32,
    pub(crate) blend: Blend,
    pub(crate) mask: Option<Mask>,
}
impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}
impl Layer {
    /// A layer overriding those under it, wholly, on every node (what a
    /// layer is until set).
    pub fn new() -> Self {
        Self {
            weight: 1.,
            blend: Blend::Override,
            mask: None,
        }
    }
    /// A layer adding to those under it, wholly, on every node.
    pub fn additive() -> Self {
        Self {
            blend: Blend::Additive,
            ..Self::new()
        }
    }
    /// It at `weight`: 0 none of it, 1 all (an additive layer may go past).
    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = if weight.is_finite() {
            weight.max(0.)
        } else {
            0.
        };
        self
    }
    /// It moving only what `mask` says, as much as it says.
    pub fn mask(mut self, mask: Mask) -> Self {
        self.mask = Some(mask);
        self
    }
}

/// How a clip plays (`World::play`): on which layer, crossfading from what
/// played there over how long, how fast, and whether it loops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub(crate) layer: usize,
    pub(crate) fade: f32,
    pub(crate) speed: f32,
    pub(crate) looping: bool,
    pub(crate) from: f32,
}
impl Default for Play {
    fn default() -> Self {
        Self::new()
    }
}
impl Play {
    /// On layer 0, at once, at its speed, looping, from its start.
    pub fn new() -> Self {
        Self {
            layer: 0,
            fade: 0.,
            speed: 1.,
            looping: true,
            from: 0.,
        }
    }
    /// On layer `layer` (counted from 0, the lowest).
    pub fn layer(mut self, layer: usize) -> Self {
        self.layer = layer;
        self
    }
    /// Crossfading from what its layer played over `seconds`.
    pub fn fade(mut self, seconds: f32) -> Self {
        self.fade = if seconds.is_finite() {
            seconds.max(0.)
        } else {
            0.
        };
        self
    }
    /// At `speed` times its own (negative plays it backwards).
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = if speed.is_finite() { speed } else { 1. };
        self
    }
    /// Once, held at its end (its start, played backwards) when done.
    pub fn once(mut self) -> Self {
        self.looping = false;
        self
    }
    /// From `seconds` into it.
    pub fn from(mut self, seconds: f32) -> Self {
        self.from = if seconds.is_finite() { seconds } else { 0. };
        self
    }
}

/// An animator of a world (`World::add_animator`): stable while it lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnimatorId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// What an animator did since it was last asked (`World::animator_update`).
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimatorUpdate {
    /// Its clips' events passed, in the order they were.
    pub events: Vec<Fired>,
}
/// A clip's event passed (`Clip::with_event`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    /// The layer playing the clip.
    pub layer: usize,
    /// The event's name.
    pub name: Arc<str>,
}

/// The longest a frame's step is taken to be: after a longer gap (its view
/// hidden) an animation goes on where it was rather than jumping.
pub(crate) const MAX_STEP: f32 = 0.25;

/// A clip playing on a layer.
struct State {
    clip: Clip,
    /// Seconds into it.
    time: f32,
    speed: f32,
    looping: bool,
    /// Its weight in its layer, going from `from` to `to` over `length`
    /// seconds, `elapsed` of them gone.
    from: f32,
    to: f32,
    elapsed: f32,
    length: f32,
    /// Where its time falls among its clip's tracks, by track.
    between: Vec<Between>,
    /// Not yet stepped: its first step takes no time, posing its start.
    fresh: bool,
}
impl State {
    fn weight(&self) -> f32 {
        if self.elapsed >= self.length {
            self.to
        } else {
            self.from + (self.to - self.from) * (self.elapsed / self.length)
        }
    }
    /// Whether it still changes as time goes on.
    fn moving(&self) -> bool {
        let duration = self.clip.duration();
        let running = self.speed != 0.
            && duration > 0.
            && (self.looping
                || (self.speed > 0. && self.time < duration)
                || (self.speed < 0. && self.time > 0.));
        running || self.elapsed < self.length
    }
    /// `dt` seconds on (`speed` the animator's): its events passed onto
    /// `fired`.
    fn step(&mut self, dt: f32, speed: f32, layer: usize, fired: &mut Vec<Fired>) {
        let first = std::mem::replace(&mut self.fresh, false);
        let dt = if first { 0. } else { dt };
        self.elapsed = (self.elapsed + dt).min(self.length.max(0.));
        let duration = self.clip.duration();
        let before = self.time;
        let delta = dt * self.speed * speed;
        let mut after = before + delta;
        let events = &self.clip.0.events;
        let mut fire = |from: f32, to: f32, inclusive_from: bool| {
            // Events in (from, to], or [from, to] for a start; backwards
            // when to < from.
            let (low, high) = if from <= to { (from, to) } else { (to, from) };
            let forward = from <= to;
            let mut each = |(t, name): &(f32, Arc<str>)| {
                let past_low = if forward {
                    *t > low || (inclusive_from && *t == low)
                } else {
                    *t >= low
                };
                let before_high = if forward {
                    *t <= high
                } else {
                    *t < high || (inclusive_from && *t == high)
                };
                if past_low && before_high {
                    fired.push(Fired {
                        layer,
                        name: name.clone(),
                    });
                }
            };
            if forward {
                events.iter().for_each(&mut each);
            } else {
                events.iter().rev().for_each(&mut each);
            }
        };
        if duration <= 0. {
            if first {
                fire(0., 0., true);
            }
            return;
        }
        if self.looping {
            if after >= duration || after < 0. {
                // Round once (a step longer than the clip passes its
                // events once).
                if delta > 0. {
                    fire(before, duration, first);
                    after = after.rem_euclid(duration);
                    fire(0., after, true);
                } else {
                    fire(before, 0., first);
                    after = after.rem_euclid(duration);
                    fire(duration, after, true);
                }
            } else {
                fire(before, after, first);
            }
        } else {
            after = after.clamp(0., duration);
            if after != before || first {
                fire(before, after, first);
            }
        }
        self.time = after;
    }
}

/// A layer as it plays.
pub(crate) struct LayerState {
    pub(crate) layer: Layer,
    /// How much of it each node takes, by node index (its mask's).
    mask: Option<Vec<f32>>,
    /// The clips playing on it, the newest last.
    states: Vec<State>,
}

/// An animator: its model's nodes, their rest, its layers.
pub(crate) struct Animator {
    /// The placed model's nodes, by node index.
    pub(crate) nodes: Vec<Option<ObjectId>>,
    /// Their places when it was made: what a node no layer moves takes.
    pub(crate) rest: Vec<Transform>,
    /// Nodes any clip it has played moves, each once: those it poses.
    pub(crate) posed: Vec<u32>,
    /// The pose being made, by node index.
    pub(crate) pose: Vec<Transform>,
    pub(crate) layers: Vec<LayerState>,
    /// Its speed, on its clips' own (0 pauses it).
    pub(crate) speed: f32,
    /// What happened since the app last asked.
    pub(crate) update: AnimatorUpdate,
    /// Changed by a call since its last frame: posed again by the next.
    pub(crate) dirty: bool,
}
impl Animator {
    pub(crate) fn new(nodes: Vec<Option<ObjectId>>, rest: Vec<Transform>) -> Self {
        Self {
            pose: rest.clone(),
            nodes,
            rest,
            posed: Vec::new(),
            layers: Vec::new(),
            speed: 1.,
            update: AnimatorUpdate::default(),
            dirty: false,
        }
    }
    /// Layer `index`, made (as `Layer::new`) with those under it if it's
    /// not yet.
    fn layer_mut(&mut self, index: usize) -> &mut LayerState {
        while self.layers.len() <= index {
            self.layers.push(LayerState {
                layer: Layer::new(),
                mask: None,
                states: Vec::new(),
            });
        }
        &mut self.layers[index]
    }
    /// Layer `index` as `layer`, its mask's weights `mask` by node.
    pub(crate) fn set_layer(&mut self, index: usize, layer: Layer, mask: Option<Vec<f32>>) {
        let state = self.layer_mut(index);
        state.layer = layer;
        state.mask = mask;
        self.dirty = true;
    }
    /// Play `clip` as `play` says.
    pub(crate) fn play(&mut self, clip: &Clip, play: Play) {
        let nodes = self.nodes.len() as u32;
        for &node in clip.0.nodes.iter().filter(|&&n| n < nodes) {
            if let Err(at) = self.posed.binary_search(&node) {
                self.posed.insert(at, node);
            }
        }
        let layer = self.layer_mut(play.layer);
        let duration = clip.duration();
        let time = if duration > 0. {
            if play.looping {
                play.from.rem_euclid(duration)
            } else {
                play.from.clamp(0., duration)
            }
        } else {
            0.
        };
        // Playing what it plays now: as it says, from where it is.
        if let Some(last) = layer.states.last_mut()
            && last.clip.same(clip)
            && last.to == 1.
        {
            last.speed = play.speed;
            last.looping = play.looping;
            self.dirty = true;
            return;
        }
        let fade = play.fade;
        if fade > 0. {
            for state in &mut layer.states {
                state.from = state.weight();
                state.to = 0.;
                state.elapsed = 0.;
                state.length = fade;
            }
        } else {
            layer.states.clear();
        }
        let (from, length) = if fade > 0. && !layer.states.is_empty() {
            (0., fade)
        } else {
            (1., 0.)
        };
        layer.states.push(State {
            clip: clip.clone(),
            time,
            speed: play.speed,
            looping: play.looping,
            from,
            to: 1.,
            elapsed: 0.,
            length,
            between: vec![Between::default(); clip.0.tracks.len()],
            fresh: true,
        });
        self.dirty = true;
    }
    /// Fade layer `index`'s clips out over `fade` seconds (at once if 0).
    pub(crate) fn stop(&mut self, index: usize, fade: f32) {
        let Some(layer) = self.layers.get_mut(index) else {
            return;
        };
        if fade > 0. {
            for state in &mut layer.states {
                state.from = state.weight();
                state.to = 0.;
                state.elapsed = 0.;
                state.length = fade;
            }
        } else {
            layer.states.clear();
        }
        self.dirty = true;
    }
    /// Whether its pose changes as time goes on.
    pub(crate) fn playing(&self) -> bool {
        self.speed != 0.
            && self
                .layers
                .iter()
                .any(|layer| layer.states.iter().any(State::moving))
    }
    /// Whether a frame should pose it: it plays, or a call changed it.
    pub(crate) fn due(&self) -> bool {
        self.dirty || self.playing()
    }
    /// `dt` seconds on: its clips' times and fades, its events noted.
    pub(crate) fn step(&mut self, dt: f32) {
        let speed = self.speed;
        let fired = &mut self.update.events;
        for (index, layer) in self.layers.iter_mut().enumerate() {
            for state in &mut layer.states {
                state.step(dt, speed, index, fired);
            }
            // Faded out: gone.
            layer
                .states
                .retain(|s| !(s.to == 0. && s.elapsed >= s.length));
        }
    }
    /// Its pose as its layers make it now, into `pose` (its nodes', by
    /// index; those in `posed` alone are written).
    pub(crate) fn evaluate(&mut self) {
        let pose = &mut self.pose;
        for &node in &self.posed {
            pose[node as usize] = self.rest[node as usize];
        }
        for layer in &mut self.layers {
            let weight = layer.layer.weight;
            let total: f32 = layer.states.iter().map(State::weight).sum();
            if weight <= 0. || total <= 0. || layer.states.is_empty() {
                continue;
            }
            let mask = layer.mask.as_deref();
            match layer.layer.blend {
                Blend::Override => {
                    // How much of the layer shows: its weight, less while
                    // its clips fade out to nothing.
                    let alpha = (weight * total.min(1.)).min(1.);
                    let only = layer.states.len() == 1;
                    let mut sum = 0.;
                    for state in &mut layer.states {
                        let share = state.weight() / total;
                        sum += share;
                        find(state);
                        let (between, clip) = (&state.between, &state.clip.0);
                        if only && alpha >= 1. && mask.is_none() {
                            for channel in &clip.channels {
                                if let Some(local) = pose.get_mut(channel.node) {
                                    set(local, channel, channel.at(between[channel.track]));
                                }
                            }
                            continue;
                        }
                        // Lerped in turn, each by its share of what's made
                        // so far: the layer's weighted mean, `alpha` of it
                        // over what was under it.
                        for channel in &clip.channels {
                            let Some(local) = pose.get_mut(channel.node) else {
                                continue;
                            };
                            let a = alpha * mask.map_or(1., |m| m[channel.node]);
                            if a <= 0. {
                                continue;
                            }
                            let t = a * share / (1. - a + a * sum);
                            let value = channel.at(between[channel.track]);
                            mix(local, channel, value, t);
                        }
                    }
                }
                Blend::Additive => {
                    for state in &mut layer.states {
                        let w = weight * state.weight();
                        find(state);
                        let between = &state.between;
                        for channel in &state.clip.0.channels {
                            let Some(local) = pose.get_mut(channel.node) else {
                                continue;
                            };
                            let a = w * mask.map_or(1., |m| m[channel.node]);
                            if a <= 0. {
                                continue;
                            }
                            let value = channel.at(between[channel.track]);
                            add(local, channel, value, channel.first(), a);
                        }
                    }
                }
            }
        }
        self.dirty = false;
    }
}

/// Where `state`'s time falls among its clip's tracks, from where it last
/// did.
#[inline]
fn find(state: &mut State) {
    let tracks = &state.clip.0.tracks;
    for (between, times) in state.between.iter_mut().zip(tracks) {
        *between = Between::after(times, state.time, *between);
    }
}

fn v3([x, y, z, _]: [f32; 4]) -> Vec3 {
    vec3(x, y, z)
}
fn quat([x, y, z, w]: [f32; 4]) -> Quat {
    Quat { x, y, z, w }
}
fn array(q: Quat) -> [f32; 4] {
    [q.x, q.y, q.z, q.w]
}
/// `local`'s part `channel` moves, `value`.
#[inline]
fn set(local: &mut Transform, channel: &Channel, value: [f32; 4]) {
    match channel.path {
        Path::Translation => local.position = v3(value),
        Path::Rotation => local.rotation = quat(value),
        Path::Scale => local.scale = v3(value),
        Path::Weight(_) => {}
    }
}
/// `local`'s part `channel` moves, `t` of the way to `value`: rotations
/// along the shorter arc.
#[inline]
fn mix(local: &mut Transform, channel: &Channel, value: [f32; 4], t: f32) {
    let lerp = |a: Vec3, b: Vec3| a + (b - a) * t;
    match channel.path {
        Path::Translation => local.position = lerp(local.position, v3(value)),
        Path::Scale => local.scale = lerp(local.scale, v3(value)),
        Path::Rotation => local.rotation = quat(nlerp(array(local.rotation), value, t)),
        Path::Weight(_) => {}
    }
}
/// `local`'s part `channel` moves, with `a` of how far `value` is from
/// `first` added: positions moved by the difference, rotations turned
/// after by it, scales multiplied by the ratio.
#[inline]
fn add(local: &mut Transform, channel: &Channel, value: [f32; 4], first: [f32; 4], a: f32) {
    match channel.path {
        Path::Translation => local.position = local.position + (v3(value) - v3(first)) * a,
        Path::Scale => {
            let ratio = |v: f32, f: f32| if f != 0. { v / f } else { 1. };
            let [x, y, z, _] = value;
            let s = vec3(ratio(x, first[0]), ratio(y, first[1]), ratio(z, first[2]));
            let s = Vec3::ONE + (s - Vec3::ONE) * a;
            local.scale = local.scale.scaled(s);
        }
        Path::Rotation => {
            let [x, y, z, w] = first;
            let inverse = Quat {
                x: -x,
                y: -y,
                z: -z,
                w,
            };
            let delta = array(inverse * quat(value));
            let delta = if a == 1. {
                delta
            } else {
                slerp([0., 0., 0., 1.], delta, a)
            };
            local.rotation = (local.rotation * quat(delta)).normalized();
        }
        Path::Weight(_) => {}
    }
}
/// `t` of the way from `a` to `b` (both at length 1), the shorter way,
/// at length 1.
#[inline]
fn nlerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let t = if dot < 0. { -t } else { t };
    let q: [f32; 4] = std::array::from_fn(|i| a[i] * (1. - t.abs()) + b[i] * t);
    let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if length > 1e-12 {
        q.map(|v| v / length)
    } else {
        a
    }
}
