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
        self.entries.push((node, unit(weight), false));
        self
    }
    /// It with `node` and every node under it taking `weight` of the layer
    /// (0 to 1).
    pub fn below(mut self, node: ObjectId, weight: f32) -> Self {
        self.entries.push((node, unit(weight), true));
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

/// `weight` in 0 to 1, none if it's not a number.
fn unit(weight: f32) -> f32 {
    if weight.is_nan() {
        0.
    } else {
        weight.clamp(0., 1.)
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
    /// How far its root motion's node went (`World::set_root_motion`), in
    /// the model's frame: taken out of its pose, for the app to move the
    /// model by. The sum of each frame's since last asked.
    pub moved: Vec3,
    /// How far it turned about the model's up (y), in radians, counter-
    /// clockwise looking down: taken out of its pose as `moved` is.
    pub turned: f32,
}

/// Motion an animator takes out of a node's pose and gives the app
/// (`World::set_root_motion`): a walk's steps moving the character, not its
/// hips away from it. Its ground plane (x and z of the model) by default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotion {
    pub(crate) node: ObjectId,
    pub(crate) vertical: bool,
    pub(crate) turn: bool,
    pub(crate) apply: Option<ObjectId>,
}
impl RootMotion {
    /// `node`'s motion along the model's x and z (its up is y).
    pub fn new(node: ObjectId) -> Self {
        Self {
            node,
            vertical: false,
            turn: false,
            apply: None,
        }
    }
    /// Its up and down too (a jump's rise moving the character).
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
    /// Its turn about the model's up too.
    pub fn turn(mut self) -> Self {
        self.turn = true;
        self
    }
    /// Moved and turned by the animator itself, the frame it's taken:
    /// `object` (the placed model's root, as a rule) goes as far along its
    /// own facing and at its own scale.
    pub fn apply_to(mut self, object: ObjectId) -> Self {
        self.apply = Some(object);
        self
    }
}
/// A root motion's node by index, and its parent's turn from the model's
/// frame (to measure it in the model's frame, where up is y).
pub(crate) struct Root {
    pub(crate) motion: RootMotion,
    pub(crate) node: usize,
    pub(crate) parent: Quat,
}
/// A clip's event passed (`Clip::with_event`).
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    /// The layer playing the clip.
    pub layer: usize,
    /// The event's name.
    pub name: Arc<str>,
}

/// The most layers an animator has (`World::set_layer`, `Play::layer`):
/// layers 0 to 15.
pub const MAX_LAYERS: usize = 16;

/// Why an animator refused a call.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimatorError {
    /// This layer is past its last (`MAX_LAYERS`).
    Layer(usize),
}
impl std::fmt::Display for AnimatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layer(layer) => write!(
                f,
                "layer {layer} is past an animator's last ({})",
                MAX_LAYERS - 1
            ),
        }
    }
}
impl std::error::Error for AnimatorError {}

/// The most morph targets a node's weights are animated for: a weight
/// channel naming one past it (a file's) is left out.
const MORPH_TARGETS: usize = 4096;

/// The most events an animator keeps for its app: past it, the older
/// half goes (as zgx-audio's ended voices).
pub(crate) const EVENTS: usize = 65_536;

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
    /// Whether its clip moves any node of the model.
    drives: bool,
    /// A fade begun since its last step: the next takes none of it.
    held: bool,
    /// Its time before its last step, and whether that step went round
    /// (1 forward, -1 backward): its root motion's span.
    before: f32,
    wrapped: i8,
    /// Its clip's channels moving the root motion's node (translation,
    /// rotation), found for that node.
    root: Option<(usize, [Option<usize>; 2])>,
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
    fn step(&mut self, dt: f32, speed: f32, layer: usize, mut fired: Option<&mut Vec<Fired>>) {
        let first = std::mem::replace(&mut self.fresh, false);
        let dt = if first { 0. } else { dt };
        // A fade starts with the frame after it was asked for, as a new
        // clip does: the step before it was asked for isn't the fade's.
        let fading = if std::mem::replace(&mut self.held, false) {
            0.
        } else {
            dt
        };
        self.elapsed = (self.elapsed + fading).min(self.length.max(0.));
        let duration = self.clip.duration();
        let before = self.time;
        self.before = before;
        self.wrapped = 0;
        // Bounded, and a number: a product too great for a float (each
        // factor finite) would make its time NaN for good.
        let delta = (dt * self.speed * speed).clamp(-1e6, 1e6);
        let delta = if delta.is_nan() { 0. } else { delta };
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
                if past_low
                    && before_high
                    && let Some(fired) = fired.as_deref_mut()
                {
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
                    self.wrapped = 1;
                } else {
                    fire(before, 0., first);
                    after = after.rem_euclid(duration);
                    fire(duration, after, true);
                    self.wrapped = -1;
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
    /// How far `root`'s node went in its last step, in the model's frame,
    /// and how far it turned about y.
    fn root_motion(&mut self, root: &Root) -> (Vec3, f32) {
        let clip = &self.clip.0;
        let found = match self.root {
            Some((node, found)) if node == root.node => found,
            _ => {
                let of = |path: Path| {
                    clip.channels
                        .iter()
                        .position(|c| c.node == root.node && c.path == path)
                };
                let found = [of(Path::Translation), of(Path::Rotation)];
                self.root = Some((root.node, found));
                found
            }
        };
        let (before, after, duration) = (self.before, self.time, clip.duration);
        let at = |channel: usize, time: f32| {
            let channel = &clip.channels[channel];
            channel.sample(&clip.tracks[channel.track], time)
        };
        // From `before` to `after`, round the end once if it went round.
        let span = |channel: usize, f: &dyn Fn([f32; 4], [f32; 4]) -> Vec3| match self.wrapped {
            1 => {
                f(at(channel, before), at(channel, duration))
                    + f(at(channel, 0.), at(channel, after))
            }
            -1 => {
                f(at(channel, before), at(channel, 0.))
                    + f(at(channel, duration), at(channel, after))
            }
            _ => f(at(channel, before), at(channel, after)),
        };
        let moved = found[0].map_or(Vec3::ZERO, |channel| {
            span(channel, &|a, b| root.parent.rotate(v3(b) - v3(a)))
        });
        let turned = found[1].filter(|_| root.motion.turn).map_or(0., |channel| {
            let yaw = |q: [f32; 4]| twist(root.parent * quat(q));
            span(channel, &|a, b| vec3(wrap(yaw(b) - yaw(a)), 0., 0.)).x
        });
        (moved, turned)
    }
}
/// `q`'s turn about y, in radians (its twist, by swing-twist).
pub(crate) fn twist(q: Quat) -> f32 {
    2. * q.y.atan2(q.w)
}
/// `angle` in (-π, π].
fn wrap(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a == -PI { PI } else { a }
}

/// A layer as it plays.
pub(crate) struct LayerState {
    pub(crate) layer: Layer,
    /// How much of it each node takes, by node index (its mask's).
    mask: Option<Vec<f32>>,
    /// Whether any of it shows: it has weight, and its mask takes some.
    shows: bool,
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
    /// Its root motion, if it takes one (`World::set_root_motion`).
    pub(crate) root: Option<Root>,
    /// The root motion its last pose took out: moved, turned.
    pub(crate) motion: (Vec3, f32),
    /// The placed model's root.
    pub(crate) model: Option<ObjectId>,
    /// Whether its app has asked what it did (`World::animator_update`):
    /// until then it keeps no events.
    pub(crate) listening: bool,
    /// The frame it last stepped at while its clips ran on with nothing
    /// showing (a layer at no weight): it rests, and the next frame posing
    /// it steps it on from there, so it comes back in step.
    pub(crate) parked: Option<std::time::Instant>,
    /// Each node's parent among the model's nodes (by index), and for one
    /// whose parent isn't, that parent: where IK finds the world.
    pub(crate) parents: Vec<Option<usize>>,
    pub(crate) above: Vec<Option<ObjectId>>,
    /// The world's matrix above each such node this frame (IK's).
    pub(crate) outer: Vec<Option<crate::Mat4>>,
    /// Its IK hook (`World::set_ik`).
    pub(crate) ik: Option<Ik>,
    /// The world's `ordering` when its IK's parents and its layers' masks
    /// were found from its hierarchy: found again once it moves.
    pub(crate) bound: u64,
    /// Each node's meshes (sharing its morph weights), by node index.
    pub(crate) meshes: Vec<Vec<ObjectId>>,
    /// Nodes any clip it has played moves the morph weights of, each once.
    pub(crate) morphed: Vec<u32>,
    /// Their weights as posed, and at rest, by node index.
    pub(crate) weights: Vec<Vec<f32>>,
    pub(crate) weights_rest: Vec<Vec<f32>>,
}
/// An IK hook: called with the blended pose each frame that poses it.
pub(crate) type Ik = Box<dyn FnMut(&mut crate::ik::Pose<'_>)>;
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
            root: None,
            motion: (Vec3::ZERO, 0.),
            model: None,
            listening: false,
            parked: None,
            parents: Vec::new(),
            above: Vec::new(),
            outer: Vec::new(),
            ik: None,
            bound: 0,
            meshes: Vec::new(),
            morphed: Vec::new(),
            weights: Vec::new(),
            weights_rest: Vec::new(),
        }
    }
    /// Layer `index`, made (as `Layer::new`) with those under it if it's
    /// not yet.
    fn layer_mut(&mut self, index: usize) -> Result<&mut LayerState, AnimatorError> {
        if index >= MAX_LAYERS {
            return Err(AnimatorError::Layer(index));
        }
        while self.layers.len() <= index {
            self.layers.push(LayerState {
                layer: Layer::new(),
                mask: None,
                shows: true,
                states: Vec::new(),
            });
        }
        Ok(&mut self.layers[index])
    }
    /// Layer `index` as `layer`, its mask's weights `mask` by node.
    pub(crate) fn set_layer(
        &mut self,
        index: usize,
        layer: Layer,
        mask: Option<Vec<f32>>,
    ) -> Result<(), AnimatorError> {
        let state = self.layer_mut(index)?;
        state.shows = layer.weight > 0. && mask.as_ref().is_none_or(|m| m.iter().any(|&w| w > 0.));
        state.layer = layer;
        state.mask = mask;
        self.dirty = true;
        Ok(())
    }
    /// Play `clip` as `play` says.
    pub(crate) fn play(&mut self, clip: &Clip, play: Play) -> Result<(), AnimatorError> {
        self.layer_mut(play.layer)?;
        let nodes = self.nodes.len() as u32;
        for channel in &clip.0.channels {
            let node = channel.node as u32;
            if node < nodes
                && let Err(at) = self.posed.binary_search(&node)
            {
                self.posed.insert(at, node);
            }
        }
        for channel in &clip.0.morphs {
            let (node, Path::Weight(target)) = (channel.node, channel.path) else {
                continue;
            };
            if node >= self.nodes.len() || target >= MORPH_TARGETS {
                continue;
            }
            if let Err(at) = self.morphed.binary_search(&(node as u32)) {
                self.morphed.insert(at, node as u32);
            }
            if self.weights_rest.len() <= node {
                self.weights_rest.resize(node + 1, Vec::new());
                self.weights.resize(node + 1, Vec::new());
            }
            let rest = &mut self.weights_rest[node];
            if rest.len() <= target {
                rest.resize(target + 1, 0.);
                self.weights[node].resize(target + 1, 0.);
            }
        }
        let layer = self.layer_mut(play.layer)?;
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
            return Ok(());
        }
        let fade = play.fade;
        if fade > 0. {
            for state in &mut layer.states {
                state.from = state.weight();
                state.to = 0.;
                state.elapsed = 0.;
                state.length = fade;
                state.held = true;
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
            drives: clip.0.nodes.iter().any(|&n| n < nodes),
            held: false,
            before: time,
            wrapped: 0,
            root: None,
        });
        self.dirty = true;
        Ok(())
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
                state.held = true;
            }
        } else {
            layer.states.clear();
        }
        self.dirty = true;
    }
    /// Whether it keeps anything found from the world's hierarchy.
    pub(crate) fn binds(&self) -> bool {
        self.ik.is_some() || self.layers.iter().any(|l| l.layer.mask.is_some())
    }
    /// Its layers' masks found again by `resolve`.
    pub(crate) fn rebind_masks(&mut self, resolve: impl Fn(&Mask) -> Vec<f32>) {
        for layer in &mut self.layers {
            if let Some(mask) = &layer.layer.mask {
                let weights = resolve(mask);
                layer.shows = layer.layer.weight > 0. && weights.iter().any(|&w| w > 0.);
                layer.mask = Some(weights);
            }
        }
    }
    /// Whether its pose changes as time goes on.
    /// Only what shows counts: a clip on a layer at no weight or masked
    /// to nothing, or moving no node of the model, runs on unseen.
    pub(crate) fn playing(&self) -> bool {
        self.speed != 0.
            && self
                .layers
                .iter()
                .any(|layer| layer.shows && layer.states.iter().any(|s| s.drives && s.moving()))
    }
    /// Whether its clips' times run on, seen or not.
    pub(crate) fn running(&self) -> bool {
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
    #[inline(never)]
    pub(crate) fn step(&mut self, dt: f32) {
        let speed = self.speed;
        let listening = self.listening;
        let fired = &mut self.update.events;
        for (index, layer) in self.layers.iter_mut().enumerate() {
            for state in &mut layer.states {
                state.step(dt, speed, index, listening.then_some(&mut *fired));
            }
            // Faded out: gone.
            layer
                .states
                .retain(|s| !(s.to == 0. && s.elapsed >= s.length));
        }
        // An app asking seldom keeps the newer half.
        if fired.len() > EVENTS {
            fired.drain(..fired.len() - EVENTS / 2);
        }
    }
    /// Its pose as its layers make it now, into `pose` (its nodes', by
    /// index; those in `posed` alone are written).
    #[inline(never)]
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
        if !self.morphed.is_empty() {
            self.evaluate_morphs();
        }
        self.dirty = false;
    }
    /// Its nodes' morph weights as its layers make them now, into
    /// `weights`: blended as places are, each weight as a number.
    #[inline(never)]
    fn evaluate_morphs(&mut self) {
        let weights = &mut self.weights;
        for &node in &self.morphed {
            let node = node as usize;
            weights[node].clone_from(&self.weights_rest[node]);
        }
        for layer in &mut self.layers {
            let weight = layer.layer.weight;
            let total: f32 = layer.states.iter().map(State::weight).sum();
            if weight <= 0. || total <= 0. {
                continue;
            }
            let mask = layer.mask.as_deref();
            let additive = layer.layer.blend == Blend::Additive;
            let alpha = (weight * total.min(1.)).min(1.);
            let mut sum = 0.;
            for state in &layer.states {
                let share = state.weight() / total;
                sum += share;
                let clip = &state.clip.0;
                for channel in &clip.morphs {
                    let Path::Weight(target) = channel.path else {
                        continue;
                    };
                    let Some(w) = weights
                        .get_mut(channel.node)
                        .and_then(|w| w.get_mut(target))
                    else {
                        continue;
                    };
                    let masked = mask.map_or(1., |m| m[channel.node]);
                    let value = channel.at(state.between[channel.track])[0];
                    if additive {
                        let a = weight * state.weight() * masked;
                        if a > 0. {
                            *w += (value - channel.first()[0]) * a;
                        }
                    } else {
                        let a = alpha * masked;
                        if a > 0. {
                            let t = a * share / (1. - a + a * sum);
                            *w += (value - *w) * t;
                        }
                    }
                }
            }
        }
        for &node in &self.morphed {
            for w in &mut weights[node as usize] {
                if !w.is_finite() {
                    *w = 0.;
                }
            }
        }
    }
    /// Its root motion's node's motion this step, blended as its pose is
    /// (`evaluate`'s shares), taken out of its pose and noted.
    #[inline(never)]
    pub(crate) fn take_root_motion(&mut self) {
        let pose = &mut self.pose;
        let Some(root) = self.root.as_ref().filter(|root| root.node < pose.len()) else {
            return;
        };
        let (mut moved, mut turned) = (Vec3::ZERO, 0.);
        for layer in &mut self.layers {
            let weight = layer.layer.weight;
            let total: f32 = layer.states.iter().map(State::weight).sum();
            if weight <= 0. || total <= 0. {
                continue;
            }
            let masked = layer.mask.as_deref().map_or(1., |m| m[root.node]);
            match layer.layer.blend {
                Blend::Override => {
                    let a = (weight * total.min(1.)).min(1.) * masked;
                    let mut sum = 0.;
                    for state in &mut layer.states {
                        let share = state.weight() / total;
                        sum += share;
                        if a > 0. {
                            let t = a * share / (1. - a + a * sum);
                            let (by, turn) = state.root_motion(root);
                            moved = moved + (by - moved) * t;
                            turned += (turn - turned) * t;
                        }
                    }
                }
                Blend::Additive => {
                    for state in &mut layer.states {
                        let a = weight * state.weight() * masked;
                        if a > 0. {
                            let (by, turn) = state.root_motion(root);
                            moved = moved + by * a;
                            turned += turn * a;
                        }
                    }
                }
            }
        }
        {
            // Taken out of its pose: it stays where it rests, along what's
            // taken.
            let motion = root.motion;
            let local = &mut pose[root.node];
            let rest = self.rest[root.node];
            let (to_model, from_model) = (root.parent, conjugate(root.parent));
            let (now, rested) = (
                to_model.rotate(local.position),
                to_model.rotate(rest.position),
            );
            let y = if motion.vertical { rested.y } else { now.y };
            local.position = from_model.rotate(vec3(rested.x, y, rested.z));
            if motion.turn {
                let turned_now = to_model * local.rotation;
                let back = twist(to_model * rest.rotation) - twist(turned_now);
                let back = Quat::from_axis_angle(Vec3::Y, back);
                local.rotation = (from_model * (back * turned_now)).normalized();
            }
            if !motion.vertical {
                moved.y = 0.;
            }
            self.motion = (moved, turned);
            self.update.moved = self.update.moved + moved;
            self.update.turned += turned;
        }
    }
}
/// `q`'s inverse, it being at length 1.
fn conjugate(q: Quat) -> Quat {
    Quat {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
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
