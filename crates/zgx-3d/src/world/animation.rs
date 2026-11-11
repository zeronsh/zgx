//! A world's animators (`World::add_animator`): played by its frames' time.
use super::{Inner, Settle, Slot, World, claim, release};
use crate::{
    Clip, Mat4, Placed, Quat, Transform, Vec3,
    animator::{
        Animator, AnimatorError, AnimatorId, AnimatorUpdate, Layer, MAX_STEP, Play, Root,
        RootMotion,
    },
};
use std::time::Instant;

/// A world's animators, and the time of the frame that last stepped them.
#[derive(Default)]
pub(crate) struct Animators {
    slots: Vec<Slot<Animator>>,
    free: usize,
    /// The last frame's time while any plays; `None` while none does, so
    /// the next to play starts from its first frame.
    last: Option<Instant>,
}

/// Step and pose `world`'s animators for its frame at `time`, before it's
/// drawn (nothing for a world without one).
#[inline]
pub(crate) fn frame(world: &std::cell::RefCell<Inner>, time: Instant) {
    let mut inner = world.borrow_mut();
    if !inner.animators.slots.is_empty() {
        advance(&mut inner, time);
    }
}

/// Step and pose `inner`'s animators to a frame at `time`, writing the
/// nodes that moved: whether any did. Inside a frame: what changed is
/// drawn by it, so nothing asks for another.
// Apart from the frame that calls it: a world without animators runs only
// `frame`'s check.
#[inline(never)]
pub(crate) fn advance(inner: &mut Inner, time: Instant) -> bool {
    if inner.animators.slots.is_empty() {
        return false;
    }
    let mut animators = std::mem::take(&mut inner.animators);
    let dt = animators.last.map_or(0., |last| {
        time.saturating_duration_since(last)
            .as_secs_f32()
            .min(MAX_STEP)
    });
    let (mut changed, mut playing) = (false, false);
    for animator in animators.slots.iter_mut().filter_map(|s| s.value.as_mut()) {
        if animator.due() {
            // Back from resting while its clips ran on unseen: on from
            // where it stopped, in step with the frames' time.
            let dt = animator.parked.take().map_or(dt, |parked| {
                time.saturating_duration_since(parked).as_secs_f32()
            });
            if animator.bound != inner.ordering && animator.binds() {
                rebind(inner, animator);
            }
            animator.step(dt);
            animator.evaluate();
            // Root motion apart: an animator without it runs none of it.
            let root = animator.root.is_some();
            if root {
                // Its node's parents' turn as they're turned now.
                let node = animator.root.as_ref().map(|r| r.motion.node);
                if let Some(node) = node {
                    let turn = turn_above(inner, node, animator.model);
                    animator.root.as_mut().unwrap().parent = turn;
                }
                animator.take_root_motion();
            }
            if animator.ik.is_some() {
                bend(inner, animator);
            }
            changed |= write(inner, animator);
            if root {
                changed |= carry(inner, animator);
            }
        }
        let plays = animator.playing();
        if !plays && animator.parked.is_none() && animator.running() {
            animator.parked = Some(time);
        }
        playing |= plays;
    }
    animators.last = playing.then_some(time);
    inner.animators = animators;
    if changed {
        inner.revision = inner.revision.wrapping_add(1);
        inner.meshes = inner.meshes.wrapping_add(1);
    }
    inner.unsettle(Settle::ANIMATION, playing);
    changed
}

/// `animator`'s pose into its nodes: only those whose places changed are
/// written (and stamped). Whether any was.
// Apart from `advance`: inlined there, what else its loop holds moved
// the code of this one (and its count).
#[inline(never)]
fn write(inner: &mut Inner, animator: &Animator) -> bool {
    let mut changed = false;
    for &node in &animator.posed {
        let node = node as usize;
        let Some(Some(id)) = animator.nodes.get(node) else {
            continue;
        };
        let Some(index) = inner.live(id.index, id.generation) else {
            continue;
        };
        let transform = sanitized(animator.pose[node]);
        let object = inner.objects[index].value.as_mut().unwrap();
        if object.transform == transform {
            continue;
        }
        object.transform = transform;
        let local = object.visible.then(|| matrix(&transform));
        inner.place(index, local);
        inner.stamp(index);
        changed = true;
    }
    if !animator.morphed.is_empty() {
        changed |= write_morphs(inner, animator);
    }
    changed
}
/// `animator`'s morph weights into its nodes' meshes, those that changed.
#[inline(never)]
fn write_morphs(inner: &mut Inner, animator: &Animator) -> bool {
    let mut changed = false;
    for &node in &animator.morphed {
        let weights = &animator.weights[node as usize];
        for &mesh in animator.meshes.get(node as usize).into_iter().flatten() {
            if inner.live(mesh.index, mesh.generation).is_none() {
                continue;
            }
            let entry = inner
                .morph_weights
                .entry(mesh.index)
                .or_insert((mesh.generation, Vec::new()));
            if entry.0 == mesh.generation && entry.1 == *weights {
                continue;
            }
            entry.0 = mesh.generation;
            entry.1.clone_from(weights);
            inner.stamp(mesh.index as usize);
            changed = true;
        }
    }
    changed
}

/// Each of `nodes`' parent among them (by index), and for one whose
/// parent isn't, that parent.
fn hierarchy(
    inner: &Inner,
    nodes: &[Option<crate::ObjectId>],
) -> (Vec<Option<usize>>, Vec<Option<crate::ObjectId>>) {
    let at: std::collections::HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| Some(((*node)?, i)))
        .collect();
    let parent = |node: &Option<crate::ObjectId>| {
        let id = (*node)?;
        let index = inner.live(id.index, id.generation)?;
        inner.objects[index].value.as_ref()?.parent
    };
    let parents = nodes
        .iter()
        .map(|node| parent(node).and_then(|p| at.get(&p).copied()))
        .collect();
    let above = nodes
        .iter()
        .map(|node| parent(node).filter(|p| !at.contains_key(p)))
        .collect();
    (parents, above)
}
/// How much of a layer each of `nodes` takes by `mask`, as the world's
/// hierarchy is now.
fn resolve(inner: &Inner, nodes: &[Option<crate::ObjectId>], mask: &crate::Mask) -> Vec<f32> {
    let (parents, _) = hierarchy(inner, nodes);
    mask.resolve(nodes, |i| parents[i])
}
/// The turn of `node`'s parent and those above it, up to `model` (the
/// placed model's root), as they're turned now.
fn turn_above(inner: &Inner, node: crate::ObjectId, model: Option<crate::ObjectId>) -> Quat {
    let mut turn = Quat::IDENTITY;
    let object = |id: crate::ObjectId| {
        let index = inner.live(id.index, id.generation)?;
        inner.objects[index].value.as_ref()
    };
    let mut at = object(node).and_then(|o| o.parent);
    for _ in 0..64 {
        let Some(id) = at.filter(|id| Some(*id) != model) else {
            break;
        };
        let Some(above) = object(id) else {
            break;
        };
        turn = above.transform.rotation * turn;
        at = above.parent;
    }
    turn.normalized()
}
/// What `animator` keeps from the world's hierarchy (its IK's parents,
/// its layers' masks) found again: a node moved under another since.
#[cold]
#[inline(never)]
fn rebind(inner: &Inner, animator: &mut Animator) {
    if animator.ik.is_some() {
        (animator.parents, animator.above) = hierarchy(inner, &animator.nodes);
    }
    let nodes = animator.nodes.clone();
    animator.rebind_masks(|mask| resolve(inner, &nodes, mask));
    animator.bound = inner.ordering;
}

/// `animator`'s pose bent by its IK hook (`World::set_ik`), the world
/// above its model as it is now.
#[inline(never)]
fn bend(inner: &Inner, animator: &mut Animator) {
    let Some(mut hook) = animator.ik.take() else {
        return;
    };
    let outer = &mut animator.outer;
    outer.clear();
    outer.extend(
        animator
            .above
            .iter()
            .map(|above| above.and_then(|id| placed_at(inner, id))),
    );
    let mut pose = crate::ik::Pose {
        nodes: &animator.nodes,
        parents: &animator.parents,
        outer: &animator.outer,
        locals: &mut animator.pose,
        posed: &mut animator.posed,
    };
    // A hook calling the world finds it borrowed: in a debug build, said so.
    #[cfg(debug_assertions)]
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook(&mut pose))) {
        let said = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("");
        if said.contains("already borrowed") || said.contains("already mutably borrowed") {
            panic!(
                "an IK hook called its world, which is borrowed while the hook runs (inside the \
                 frame): share what it reads by an Rc<Cell<_>>, and change the world outside it"
            );
        }
        std::panic::resume_unwind(panic);
    }
    #[cfg(not(debug_assertions))]
    hook(&mut pose);
    animator.ik = Some(hook);
}
/// Object `id`'s matrix in the world from its kept local matrices: `None`
/// where it or one above is hidden or gone.
fn placed_at(inner: &Inner, id: crate::ObjectId) -> Option<Mat4> {
    let mut matrix = Mat4::IDENTITY;
    let mut at = Some(id);
    for _ in 0..64 {
        let Some(id) = at else {
            return Some(matrix);
        };
        let index = inner.live(id.index, id.generation)?;
        matrix = inner.locals.get(index).copied().flatten()? * matrix;
        at = inner.objects[index].value.as_ref()?.parent;
    }
    None
}

/// `animator`'s root motion this frame applied to the object it says
/// (`RootMotion::apply_to`), along its facing at its scale: whether it moved.
#[inline(never)]
fn carry(inner: &mut Inner, animator: &Animator) -> bool {
    let Some(id) = animator.root.as_ref().and_then(|root| root.motion.apply) else {
        return false;
    };
    let (moved, turned) = animator.motion;
    let Some(index) = inner.live(id.index, id.generation) else {
        return false;
    };
    if moved == Vec3::ZERO && turned == 0. {
        return false;
    }
    let object = inner.objects[index].value.as_mut().unwrap();
    let t = object.transform;
    let transform = sanitized(Transform {
        position: t.position + t.rotation.rotate(moved.scaled(t.scale)),
        rotation: t.rotation * Quat::from_axis_angle(Vec3::Y, turned),
        ..t
    });
    object.transform = transform;
    let local = object.visible.then(|| matrix(&transform));
    inner.place(index, local);
    inner.stamp(index);
    true
}

/// `t` as `Transform::sanitized` makes it, in fewer steps where it's all
/// numbers (as a clip's poses are).
#[inline]
fn sanitized(t: Transform) -> Transform {
    let (p, r, s) = (t.position, t.rotation, t.scale);
    let sum = p.x + p.y + p.z + r.x + r.y + r.z + r.w + s.x + s.y + s.z;
    if !sum.is_finite() {
        return t.sanitized();
    }
    Transform {
        rotation: r.normalized(),
        ..t
    }
}
/// `t`'s matrix, its rotation at length 1: `Transform::matrix`'s, from
/// the rotation's products rather than three vectors turned.
#[inline]
fn matrix(t: &Transform) -> Mat4 {
    let Quat { x, y, z, w } = t.rotation;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, yy, zz) = (x * x2, y * y2, z * z2);
    let (xy, xz, yz) = (x * y2, x * z2, y * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    let (s, p) = (t.scale, t.position);
    Mat4 {
        columns: [
            [(1. - (yy + zz)) * s.x, (xy + wz) * s.x, (xz - wy) * s.x, 0.],
            [(xy - wz) * s.y, (1. - (xx + zz)) * s.y, (yz + wx) * s.y, 0.],
            [(xz + wy) * s.z, (yz - wx) * s.z, (1. - (xx + yy)) * s.z, 0.],
            [p.x, p.y, p.z, 1.],
        ],
    }
}

impl World {
    /// An animator of `placed` (`World::place_model`): it plays the
    /// model's clips on its nodes, posed by the world's frames at their
    /// time, so a video renders it the same each time. While a clip plays,
    /// the world draws frames by itself; paused, stopped or done, it
    /// draws none. Its nodes rest where they are now: a node no playing
    /// clip moves goes back there. Clips play on layer 0 until others are
    /// set (`set_layer`).
    pub fn add_animator(&self, placed: &Placed) -> AnimatorId {
        let mut inner = self.inner.borrow_mut();
        let rest = placed
            .nodes
            .iter()
            .map(|node| {
                node.and_then(|id| inner.live(id.index, id.generation))
                    .and_then(|at| inner.objects[at].value.as_ref())
                    .map_or_else(Default::default, |object| object.transform)
            })
            .collect();
        let mut animator = Animator::new(placed.nodes.clone(), rest);
        animator.model = Some(placed.root);
        // Each node's meshes, and their morph weights at rest (a node's
        // meshes share them).
        animator.meshes = placed.meshes.clone();
        animator.weights_rest = placed
            .meshes
            .iter()
            .map(|meshes| {
                let first = meshes.first();
                first
                    .and_then(|id| {
                        inner
                            .morph_weights
                            .get(&id.index)
                            .filter(|(g, _)| *g == id.generation)
                    })
                    .map(|(_, weights)| weights.clone())
                    .unwrap_or_default()
            })
            .collect();
        animator.weights = animator.weights_rest.clone();
        let animators = &mut inner.animators;
        let (index, generation) = claim(&mut animators.slots, &mut animators.free, animator);
        AnimatorId { index, generation }
    }
    /// Remove animator `id`: its nodes stay as it last posed them.
    pub fn remove_animator(&self, id: AnimatorId) {
        let mut inner = self.inner.borrow_mut();
        let animators = &mut inner.animators;
        if Self::animator_of(&mut animators.slots, id).is_none() {
            return;
        }
        release(&mut animators.slots, &mut animators.free, id.index as usize);
        if animators.slots.iter().all(|slot| slot.value.is_none()) {
            animators.slots.clear();
            animators.free = 0;
        }
        let playing = Self::any_playing(&inner);
        inner.unsettle(Settle::ANIMATION, playing);
    }
    /// Play `clip` (one of its model's) on animator `id`, as `play` says:
    /// on which layer, crossfading over how long from what played there,
    /// how fast, looping or once. Playing what its layer plays already
    /// changes only its speed and looping.
    /// Fails for a layer past the last (`MAX_LAYERS`); nothing for an
    /// animator gone.
    pub fn play(&self, id: AnimatorId, clip: &Clip, play: Play) -> Result<(), AnimatorError> {
        self.change_animator(id, |animator| animator.play(clip, play))
            .unwrap_or(Ok(()))
    }
    /// Fade out what animator `id` plays on `layer` over `fade` seconds
    /// (at once if 0): the layers under it show through.
    pub fn stop(&self, id: AnimatorId, layer: usize, fade: f32) {
        let fade = if fade.is_finite() { fade.max(0.) } else { 0. };
        self.change_animator(id, |animator| animator.stop(layer, fade));
    }
    /// Animator `id`'s layer `index` (counted from 0, the lowest) as
    /// `layer` says: its weight, mask and blend. While no layer shows
    /// anything (weight 0, a mask taking nothing) the world rests and its
    /// clips' times stand; shown again, they go on by the whole time
    /// between, in step, and a looping clip's events and root motion for
    /// that stretch come out as at most one loop's worth. Layers up to it that
    /// weren't set override wholly.
    /// Fails for a layer past the last (`MAX_LAYERS`).
    pub fn set_layer(
        &self,
        id: AnimatorId,
        index: usize,
        layer: Layer,
    ) -> Result<(), AnimatorError> {
        if index >= crate::MAX_LAYERS {
            return Err(AnimatorError::Layer(index));
        }
        let (mask, bound) = {
            let inner = self.inner.borrow();
            let Some(animator) = Self::animator_ref(&inner, id) else {
                return Ok(());
            };
            let mask = layer
                .mask
                .as_ref()
                .map(|mask| resolve(&inner, &animator.nodes, mask));
            (mask, inner.ordering)
        };
        self.change_animator(id, |animator| {
            animator.bound = bound;
            animator.set_layer(index, layer, mask)
        })
        .unwrap_or(Ok(()))
    }
    /// Animator `id`'s root motion (`None`: none, its default): the motion
    /// of a node (a character's hips or root bone) taken out of its pose
    /// and given in `animator_update`'s `moved` and `turned`, or applied
    /// to an object (`RootMotion::apply_to`), so a walk moves the character
    /// rather than its hips away from it. Blended as the pose is: a
    /// crossfade from a walk to a run speeds up as it goes. Measured in the
    /// model's frame (its up y) from the node's parent, as that is turned
    /// now.
    pub fn set_root_motion(&self, id: AnimatorId, root: Option<RootMotion>) {
        let root = {
            let inner = self.inner.borrow();
            let Some(animator) = Self::animator_ref(&inner, id) else {
                return;
            };
            root.and_then(|motion| {
                let node = animator
                    .nodes
                    .iter()
                    .position(|n| *n == Some(motion.node))?;
                let parent = turn_above(&inner, motion.node, animator.model);
                Some(Root {
                    motion,
                    node,
                    parent,
                })
            })
        };
        self.change_animator(id, |animator| animator.root = root);
    }
    /// Animator `id`'s IK hook: `hook` is called with its pose each frame
    /// it's posed, after its layers blend it and root motion is taken out,
    /// before it's drawn (and its skins follow). It reads and sets nodes'
    /// places (`Pose::local`, `set_local`, `world`), or runs the solvers
    /// in `ik`. It's called inside the frame: it must not call the world
    /// (it would find it borrowed), and holding a `World` makes a cycle:
    /// share what it reads (a target) by an `Rc<Cell<_>>`. While nothing
    /// plays, `repose` asks for a frame to call it again (a target moved).
    pub fn set_ik(&self, id: AnimatorId, hook: impl FnMut(&mut crate::ik::Pose<'_>) + 'static) {
        // Each node's parent, among the model's nodes or the object above:
        // found for IK alone, so an animator without it pays nothing (and
        // found again as the world's hierarchy changes: `rebind`).
        let (found, bound) = {
            let inner = self.inner.borrow();
            let Some(animator) = Self::animator_ref(&inner, id) else {
                return;
            };
            (hierarchy(&inner, &animator.nodes), inner.ordering)
        };
        self.change_animator(id, |animator| {
            (animator.parents, animator.above) = found;
            animator.bound = bound;
            animator.ik = Some(Box::new(hook));
        });
    }
    /// Animator `id` without its IK hook.
    pub fn clear_ik(&self, id: AnimatorId) {
        self.change_animator(id, |animator| animator.ik = None);
    }
    /// Pose animator `id` again with the next frame, its IK hook called:
    /// for a target moved while nothing it plays moves.
    pub fn repose(&self, id: AnimatorId) {
        self.change_animator(id, |animator| animator.dirty = true);
    }
    /// Animator `id`'s speed, on its clips' own: 1 by default, 0 pauses it
    /// (and the world settles), negative plays them backwards.
    pub fn set_animator_speed(&self, id: AnimatorId, speed: f32) {
        let speed = if speed.is_finite() { speed } else { 1. };
        self.change_animator(id, |animator| animator.speed = speed);
    }
    /// What animator `id` did since this was last asked: its clips' events
    /// passed, and its root motion. An animator keeps its events once
    /// this has been asked (ask once after adding it to hear its first),
    /// the newest 65,536 at most. Empty for one gone.
    pub fn animator_update(&self, id: AnimatorId) -> AnimatorUpdate {
        let mut inner = self.inner.borrow_mut();
        Self::animator_of(&mut inner.animators.slots, id)
            .map(|animator| {
                animator.listening = true;
                std::mem::take(&mut animator.update)
            })
            .unwrap_or_default()
    }
    /// Whether animator `id` plays: its pose changes as time goes on.
    pub fn animator_playing(&self, id: AnimatorId) -> bool {
        let inner = self.inner.borrow();
        Self::animator_ref(&inner, id).is_some_and(Animator::playing)
    }
    /// Step and pose its animators to a frame at `time`, as its view's
    /// frames do: for drawing it without a view (tests, a renderer of
    /// one's own). The first after none played takes no time.
    #[doc(hidden)]
    pub fn animate(&self, time: Instant) {
        let changed = advance(&mut self.inner.borrow_mut(), time);
        if changed {
            self.view.invalidate();
        }
    }
    /// `f` on animator `id`, if it lives: what it returned.
    fn change_animator<T>(&self, id: AnimatorId, f: impl FnOnce(&mut Animator) -> T) -> Option<T> {
        let mut inner = self.inner.borrow_mut();
        let animator = Self::animator_of(&mut inner.animators.slots, id)?;
        let out = f(animator);
        let playing = Self::any_playing(&inner);
        inner.unsettle(Settle::ANIMATION, playing);
        drop(inner);
        // A frame poses it.
        self.view.invalidate();
        Some(out)
    }
    fn any_playing(inner: &Inner) -> bool {
        let slots = &inner.animators.slots;
        slots
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .any(Animator::playing)
    }
    fn animator_of(slots: &mut [Slot<Animator>], id: AnimatorId) -> Option<&mut Animator> {
        let slot = slots.get_mut(id.index as usize)?;
        (slot.generation == id.generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }
    fn animator_ref(inner: &Inner, id: AnimatorId) -> Option<&Animator> {
        let slot = inner.animators.slots.get(id.index as usize)?;
        (slot.generation == id.generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Clip, Layer, Mask, Object, ObjectId, Placed, Play, Quat, RootMotion, Transform, Vec3,
        World,
        animation::{Channel, Interpolation, Path},
        animator::Blend,
        vec3,
    };
    use std::time::{Duration, Instant};

    /// A root, node `a` under it and `b` under `a`, placed as a model's.
    fn rig(world: &World) -> (Placed, ObjectId, ObjectId) {
        let root = world.add(Object::group());
        let a = world.add(Object::group().child_of(root));
        let b = world.add(Object::group().at([0., 1., 0.]).child_of(a));
        let placed = Placed {
            root,
            nodes: vec![Some(a), Some(b)],
            meshes: vec![Vec::new(), Vec::new()],
            lights: Vec::new(),
            cameras: Vec::new(),
        };
        (placed, a, b)
    }
    /// A linear channel: node, what, keys' times, values.
    type Keys = (usize, Path, Vec<f32>, Vec<[f32; 4]>);
    fn clip(channels: Vec<Keys>) -> Clip {
        let channels = channels
            .into_iter()
            .map(|(node, path, times, values)| {
                let channel = Channel {
                    node,
                    path,
                    interpolation: Interpolation::Linear,
                    track: 0,
                    values,
                };
                (times, channel)
            })
            .collect();
        Clip::new(None, channels)
    }
    fn x(x: f32) -> [f32; 4] {
        [x, 0., 0., 0.]
    }
    /// Node 0's x going `from` to `to` over a second.
    fn slide(from: f32, to: f32) -> Clip {
        clip(vec![(
            0,
            Path::Translation,
            vec![0., 1.],
            vec![x(from), x(to)],
        )])
    }
    fn turn(angle: f32) -> [f32; 4] {
        let q = Quat::from_axis_angle(Vec3::Y, angle);
        [q.x, q.y, q.z, q.w]
    }
    fn at(world: &World, id: ObjectId) -> Transform {
        world.object(id).unwrap().transform
    }
    struct Clock(Instant);
    impl Clock {
        fn frame(&self, world: &World, seconds: f32) {
            world.animate(self.0 + Duration::from_secs_f32(seconds));
        }
    }
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }
    /// About Y, in radians, of a rotation about Y.
    fn angle(q: Quat) -> f32 {
        2. * q.y.atan2(q.w)
    }

    #[test]
    fn a_clip_plays_by_the_frames_time() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.play(animator, &slide(0., 2.), Play::new()).unwrap();
        let clock = Clock(Instant::now());
        // Its first frame poses its start, whenever it comes.
        clock.frame(&world, 10.);
        assert_eq!(at(&world, a).position, vec3(0., 0., 0.));
        clock.frame(&world, 10.25);
        assert_eq!(at(&world, a).position, vec3(0.5, 0., 0.));
        // Looping: 1.1 s in is 0.1.
        for t in [10.5, 10.75, 11., 11.1] {
            clock.frame(&world, t);
        }
        assert!(close(at(&world, a).position.x, 0.2), "{:?}", at(&world, a));
    }

    #[test]
    fn an_animator_poses_as_placed_pose_does() {
        let world = World::new();
        let (placed, a, b) = rig(&world);
        let clip = clip(vec![
            (
                0,
                Path::Rotation,
                vec![0., 0.5, 2.],
                vec![turn(0.), turn(1.), turn(2.5)],
            ),
            (
                1,
                Path::Translation,
                vec![0., 2.],
                vec![[0., 1., 0., 0.], [3., 1., 2., 0.]],
            ),
            (
                1,
                Path::Scale,
                vec![0., 2.],
                vec![[1., 1., 1., 0.], [2., 1., 0.5, 0.]],
            ),
        ]);
        let animator = world.add_animator(&placed);
        world.play(animator, &clip, Play::new()).unwrap();
        let clock = Clock(Instant::now());
        let other = World::new();
        let (theirs, oa, ob) = rig(&other);
        for (i, t) in [0., 0.1, 0.3, 0.5, 0.7, 0.95, 1.2, 1.45, 1.7, 1.9]
            .into_iter()
            .enumerate()
        {
            clock.frame(&world, t);
            theirs.pose(&other, &clip, t);
            assert_eq!(at(&world, a), at(&other, oa), "frame {i}");
            assert_eq!(at(&world, b), at(&other, ob), "frame {i}");
        }
    }

    #[test]
    fn a_crossfade_blends_the_clips_by_how_far_it_has_gone() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        // Both clips away from the rest (x 0), so a fade that lost weight
        // to it would show.
        world.play(animator, &slide(1., 1.), Play::new()).unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        world
            .play(animator, &slide(3., 3.), Play::new().fade(1.))
            .unwrap();
        clock.frame(&world, 0.1);
        assert_eq!(at(&world, a).position.x, 1., "the new clip's first frame");
        clock.frame(&world, 0.35);
        assert!(close(at(&world, a).position.x, 1.5), "{:?}", at(&world, a));
        clock.frame(&world, 0.6);
        assert!(close(at(&world, a).position.x, 2.), "halfway");
        clock.frame(&world, 0.85);
        clock.frame(&world, 1.1);
        assert_eq!(at(&world, a).position.x, 3., "done: the new clip alone");
        // Stopped over a second: its first frame still all of it, then
        // fading to the rest.
        world.stop(animator, 0, 1.);
        clock.frame(&world, 1.2);
        assert_eq!(at(&world, a).position.x, 3.);
        clock.frame(&world, 1.45);
        assert!(close(at(&world, a).position.x, 2.25), "{:?}", at(&world, a));
    }

    #[test]
    fn a_layer_overrides_those_under_it_by_its_weight_and_mask() {
        let world = World::new();
        let (placed, a, b) = rig(&world);
        let animator = world.add_animator(&placed);
        let both = |v: f32| {
            clip(vec![
                (0, Path::Translation, vec![0.], vec![x(v)]),
                (1, Path::Translation, vec![0.], vec![x(v)]),
            ])
        };
        world.play(animator, &both(1.), Play::new()).unwrap();
        world
            .play(animator, &both(3.), Play::new().layer(1))
            .unwrap();
        world
            .set_layer(animator, 1, Layer::new().weight(0.25))
            .unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        assert_eq!(at(&world, a).position.x, 1.5, "a quarter of the way");
        assert_eq!(at(&world, b).position.x, 1.5);
        // Masked to `b` alone, at half.
        world
            .set_layer(animator, 1, Layer::new().mask(Mask::new().node(b, 0.5)))
            .unwrap();
        clock.frame(&world, 0.1);
        assert_eq!(at(&world, a).position.x, 1., "unmasked: under it");
        assert_eq!(at(&world, b).position.x, 2.);
        // Masked below `a`: both.
        world
            .set_layer(animator, 1, Layer::new().mask(Mask::new().below(a, 1.)))
            .unwrap();
        clock.frame(&world, 0.2);
        assert_eq!(
            (at(&world, a).position.x, at(&world, b).position.x),
            (3., 3.)
        );
        // Stopped at once: under it again.
        world.stop(animator, 1, 0.);
        clock.frame(&world, 0.3);
        assert_eq!(
            (at(&world, a).position.x, at(&world, b).position.x),
            (1., 1.)
        );
    }

    #[test]
    fn an_additive_layer_adds_how_far_its_clip_moved() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        let base = clip(vec![
            (0, Path::Rotation, vec![0.], vec![turn(0.5)]),
            (0, Path::Translation, vec![0.], vec![x(1.)]),
        ]);
        // From a quarter turn to a half, and from x 5 to 7: a quarter turn
        // and 2 at its end.
        let lean = clip(vec![
            (0, Path::Rotation, vec![0., 1.], vec![turn(1.), turn(2.)]),
            (0, Path::Translation, vec![0., 1.], vec![x(5.), x(7.)]),
        ]);
        world.play(animator, &base, Play::new()).unwrap();
        world
            .play(animator, &lean, Play::new().layer(1).once())
            .unwrap();
        world
            .set_layer(animator, 1, Layer::additive().weight(0.5))
            .unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        let pose = at(&world, a);
        assert!(
            close(angle(pose.rotation), 0.5) && pose.position.x == 1.,
            "nothing yet"
        );
        clock.frame(&world, 0.2);
        clock.frame(&world, 0.45);
        clock.frame(&world, 0.7);
        clock.frame(&world, 0.95);
        clock.frame(&world, 1.2);
        let pose = at(&world, a);
        assert!(
            close(angle(pose.rotation), 1.),
            "0.5 and half of 1: {pose:?}"
        );
        assert!(close(pose.position.x, 2.), "1 and half of 2: {pose:?}");
        assert_eq!(Blend::default(), Blend::Override);
    }

    #[test]
    fn a_world_draws_while_a_clip_plays_and_settles_after() {
        let world = World::new();
        let (placed, a, b) = rig(&world);
        let animator = world.add_animator(&placed);
        assert!(!world.view().animating(), "an animator alone plays nothing");
        world
            .play(animator, &slide(0., 2.), Play::new().once())
            .unwrap();
        assert!(world.view().animating() && world.animator_playing(animator));
        let clock = Clock(Instant::now());
        let b_stamp = || world.inner().stamps[b.index as usize];
        let stamped = b_stamp();
        for frame in 0..=4 {
            clock.frame(&world, frame as f32 * 0.25);
        }
        assert_eq!(at(&world, a).position.x, 2.);
        assert_eq!(b_stamp(), stamped, "a node it doesn't move isn't written");
        assert!(!world.view().animating() && !world.animator_playing(animator));
        // Done: frames change nothing (a world at rest asks for none).
        let revision = world.revision();
        clock.frame(&world, 1.5);
        clock.frame(&world, 2.);
        assert_eq!(world.revision(), revision);
        // Paused, a looping clip settles too.
        world.play(animator, &slide(0., 1.), Play::new()).unwrap();
        assert!(world.view().animating());
        world.set_animator_speed(animator, 0.);
        assert!(!world.view().animating());
        world.set_animator_speed(animator, 1.);
        assert!(world.view().animating());
        world.remove_animator(animator);
        assert!(!world.view().animating());
        world.play(animator, &slide(0., 1.), Play::new()).unwrap();
        assert!(
            !world.view().animating(),
            "a removed animator plays nothing"
        );
    }

    #[test]
    fn the_same_frame_times_pose_the_same() {
        let times = [0., 0.016, 0.033, 0.07, 0.4, 0.41, 0.9, 3.0, 3.016];
        let run = || {
            let world = World::new();
            let (placed, a, b) = rig(&world);
            let animator = world.add_animator(&placed);
            let clip = clip(vec![
                (
                    0,
                    Path::Rotation,
                    vec![0., 0.3, 0.8],
                    vec![turn(0.), turn(2.), turn(-1.)],
                ),
                (1, Path::Translation, vec![0., 0.8], vec![x(0.), x(3.)]),
            ]);
            world.play(animator, &clip, Play::new().speed(1.3)).unwrap();
            let clock = Clock(Instant::now());
            let mut poses = Vec::new();
            for (i, &t) in times.iter().enumerate() {
                if i == 4 {
                    world
                        .play(animator, &slide(1., 2.), Play::new().fade(0.3))
                        .unwrap();
                }
                clock.frame(&world, t);
                poses.push((at(&world, a), at(&world, b)));
            }
            poses
        };
        let (first, second) = (run(), run());
        let bits = |poses: &[(Transform, Transform)]| -> Vec<u32> {
            poses
                .iter()
                .flat_map(|(a, b)| [*a, *b])
                .flat_map(|t| {
                    let r = t.rotation;
                    [t.position.x, t.position.y, t.position.z, r.x, r.y, r.z, r.w]
                })
                .map(f32::to_bits)
                .collect()
        };
        assert_eq!(bits(&first), bits(&second));
    }

    #[test]
    fn a_pose_is_placed_as_its_transform_says() {
        let t = Transform {
            position: vec3(1., -2., 3.),
            rotation: Quat::from_euler(0.3, -1.2, 2.1),
            scale: vec3(0.5, 2., 1.5),
        };
        let (ours, theirs) = (super::matrix(&t), t.matrix());
        for (a, b) in ours
            .columns
            .iter()
            .flatten()
            .zip(theirs.columns.iter().flatten())
        {
            assert!((a - b).abs() < 1e-6, "{ours:?} {theirs:?}");
        }
        let nan = Transform {
            position: vec3(f32::NAN, 0., 0.),
            ..t
        };
        assert_eq!(super::sanitized(nan), nan.sanitized());
        assert_eq!(super::sanitized(t), t.sanitized());
    }

    /// Frames every quarter second from `from` to `to`.
    fn frames(clock: &Clock, world: &World, from: f32, to: f32) {
        let mut t = from;
        while t <= to + 1e-4 {
            clock.frame(world, t);
            t += 0.25;
        }
    }

    #[test]
    fn root_motion_moves_the_model_not_its_hips() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.set_root_motion(animator, Some(RootMotion::new(a)));
        // 2 m a loop of a second, the hips also bobbing 0.5 up.
        let walk = clip(vec![(
            0,
            Path::Translation,
            vec![0., 0.5, 1.],
            vec![[0., 0., 0., 0.], [1., 0.5, 0., 0.], [2., 0., 0., 0.]],
        )]);
        world.play(animator, &walk, Play::new()).unwrap();
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 2.5);
        let update = world.animator_update(animator);
        assert!(
            close(update.moved.x, 5.),
            "2.5 s round the loop: {update:?}"
        );
        assert_eq!(
            (update.moved.y, update.moved.z, update.turned),
            (0., 0., 0.)
        );
        let hips = at(&world, a).position;
        assert_eq!((hips.x, hips.z), (0., 0.), "where they rest");
        assert_eq!(hips.y, 0.5, "the bob stays in the pose (2.5 s is halfway)");
        clock.frame(&world, 2.75);
        assert!(
            close(world.animator_update(animator).moved.x, 0.5),
            "asked again: since"
        );
    }

    #[test]
    fn root_motion_blends_as_the_pose_does() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.set_root_motion(animator, Some(RootMotion::new(a)));
        world.play(animator, &slide(0., 1.), Play::new()).unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        world.animator_update(animator);
        world
            .play(animator, &slide(0., 3.), Play::new().fade(1.))
            .unwrap();
        // The fade's first frame is the walk's alone (speed 1); then four
        // quarter seconds at weights 0.25 to 1 of the run: speeds 1.5, 2,
        // 2.5 and 3.
        frames(&clock, &world, 0.25, 1.25);
        let moved = world.animator_update(animator).moved.x;
        assert!(close(moved, 0.25 * (1. + 1.5 + 2. + 2.5 + 3.)), "{moved}");
    }

    #[test]
    fn root_motion_turns_and_moves_an_object_along_its_facing() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let quarter = std::f32::consts::FRAC_PI_2;
        world.update(placed.root, |o| {
            o.transform.rotation = Quat::from_axis_angle(Vec3::Y, quarter);
            o.transform.scale = Vec3::ONE * 2.;
        });
        let animator = world.add_animator(&placed);
        let motion = RootMotion::new(a).turn().apply_to(placed.root);
        world.set_root_motion(animator, Some(motion));
        let walk = clip(vec![
            (0, Path::Translation, vec![0., 1.], vec![x(0.), x(1.)]),
            (
                0,
                Path::Rotation,
                vec![0., 1.],
                vec![turn(0.), turn(quarter)],
            ),
        ]);
        world.play(animator, &walk, Play::new().once()).unwrap();
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 0.25);
        let update = world.animator_update(animator);
        assert!(
            close(update.moved.x, 0.25) && close(update.turned, quarter / 4.),
            "{update:?}"
        );
        let root = at(&world, placed.root);
        // A quarter turn left: its x is the world's -z; twice the size.
        assert!(
            close(root.position.z, -0.5) && close(root.position.x, 0.),
            "{root:?}"
        );
        assert!(close(angle(root.rotation), quarter * 1.25));
        let hips = at(&world, a);
        assert!(
            close(angle(hips.rotation), 0.) && hips.position.x == 0.,
            "{hips:?}"
        );
    }

    #[test]
    fn root_motion_is_measured_in_the_models_frame() {
        let world = World::new();
        let (placed, a, b) = rig(&world);
        // A z-up file's armature: turned a quarter about x, so its y is the
        // model's -z.
        let quarter = std::f32::consts::FRAC_PI_2;
        world.update(a, |o| {
            o.transform.rotation = Quat::from_axis_angle(vec3(1., 0., 0.), -quarter)
        });
        let animator = world.add_animator(&placed);
        world.set_root_motion(animator, Some(RootMotion::new(b)));
        let forward = clip(vec![(
            1,
            Path::Translation,
            vec![0., 1.],
            vec![[0., 1., 0., 0.], [0., 2., 0., 0.]],
        )]);
        world.play(animator, &forward, Play::new().once()).unwrap();
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 0.5);
        let moved = world.animator_update(animator).moved;
        assert!(
            close(moved.z, -0.5) && close(moved.x, 0.) && moved.y == 0.,
            "{moved:?}"
        );
        let hips = at(&world, b).position;
        assert!(close(hips.y, 1.) && close(hips.z, 0.), "at rest: {hips:?}");
    }

    #[test]
    fn a_long_gap_between_frames_is_taken_as_a_short_step() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world
            .play(animator, &slide(0., 4.), Play::new().once())
            .unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        clock.frame(&world, 30.);
        assert_eq!(at(&world, a).position.x, 1., "a quarter second on");
    }

    #[test]
    fn events_are_reported_as_the_clip_passes_them() {
        let world = World::new();
        let (placed, _, _) = rig(&world);
        let animator = world.add_animator(&placed);
        let clip = slide(0., 1.)
            .with_event(0.5, "step")
            .with_event(0., "start");
        // Asked once: it keeps them from now on.
        assert!(world.animator_update(animator).events.is_empty());
        world.play(animator, &clip, Play::new()).unwrap();
        let clock = Clock(Instant::now());
        let names = |world: &World| -> Vec<String> {
            let update = world.animator_update(animator);
            update.events.iter().map(|e| e.name.to_string()).collect()
        };
        clock.frame(&world, 0.);
        assert_eq!(names(&world), ["start"]);
        clock.frame(&world, 0.2);
        clock.frame(&world, 0.4);
        assert!(names(&world).is_empty());
        clock.frame(&world, 0.5);
        clock.frame(&world, 0.6);
        assert_eq!(names(&world), ["step"], "once");
        clock.frame(&world, 0.8);
        clock.frame(&world, 1.05);
        clock.frame(&world, 1.2);
        clock.frame(&world, 1.4);
        clock.frame(&world, 1.6);
        assert_eq!(names(&world), ["start", "step"], "round again");
    }

    #[test]
    fn a_layer_at_no_weight_lets_the_world_rest_and_resumes_in_step() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.play(animator, &slide(0., 2.), Play::new()).unwrap();
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 1.);
        world
            .set_layer(animator, 0, Layer::new().weight(0.))
            .unwrap();
        clock.frame(&world, 1.25);
        assert!(!world.view().animating(), "nothing it plays shows");
        let revision = world.revision();
        frames(&clock, &world, 1.5, 3.);
        assert_eq!(world.revision(), revision, "no frame changes anything");
        // Back on 3.1 s in: where the clip would be had it played on.
        world.set_layer(animator, 0, Layer::new()).unwrap();
        assert!(world.view().animating());
        clock.frame(&world, 3.1);
        assert!(close(at(&world, a).position.x, 0.2), "{:?}", at(&world, a));
        // A mask taking nothing, and a clip moving no node of its model,
        // play nothing either.
        world
            .set_layer(animator, 0, Layer::new().mask(Mask::new()))
            .unwrap();
        clock.frame(&world, 3.2);
        assert!(!world.view().animating());
        let elsewhere = clip(vec![(
            7,
            Path::Translation,
            vec![0., 1.],
            vec![x(0.), x(1.)],
        )]);
        world
            .play(animator, &elsewhere, Play::new().layer(1))
            .unwrap();
        clock.frame(&world, 3.3);
        assert!(!world.view().animating());
    }

    #[test]
    fn events_are_kept_only_for_an_app_that_asks_and_only_so_many() {
        let world = World::new();
        let (placed, _, _) = rig(&world);
        let animator = world.add_animator(&placed);
        let mut busy = slide(0., 1.);
        for i in 0..1000 {
            busy = busy.with_event(i as f32 / 1000., "tick");
        }
        world.play(animator, &busy, Play::new()).unwrap();
        let clock = Clock(Instant::now());
        let kept = |world: &World| {
            let inner = world.inner();
            let slot = &inner.animators.slots[animator.index as usize];
            slot.value.as_ref().unwrap().update.events.len()
        };
        frames(&clock, &world, 0., 5.);
        assert_eq!(kept(&world), 0, "never asked: none kept");
        assert!(world.animator_update(animator).events.is_empty());
        frames(&clock, &world, 5.25, 80.);
        let held = kept(&world);
        assert!(held > 0 && held <= crate::animator::EVENTS, "{held}");
    }

    #[test]
    fn layers_past_the_last_are_refused() {
        let world = World::new();
        let (placed, _, _) = rig(&world);
        let animator = world.add_animator(&placed);
        let last = crate::MAX_LAYERS - 1;
        assert!(
            world
                .play(animator, &slide(0., 1.), Play::new().layer(last))
                .is_ok()
        );
        let past = world.play(animator, &slide(0., 1.), Play::new().layer(1 << 30));
        assert_eq!(past, Err(crate::AnimatorError::Layer(1 << 30)));
        assert!(world.set_layer(animator, last + 1, Layer::new()).is_err());
        let inner = world.inner();
        let slot = &inner.animators.slots[animator.index as usize];
        assert_eq!(slot.value.as_ref().unwrap().layers.len(), last + 1);
    }

    #[test]
    fn speeds_too_great_to_multiply_keep_a_pose() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        let clip = slide(0., 2.);
        world
            .play(animator, &clip, Play::new().speed(3e38))
            .unwrap();
        world.set_animator_speed(animator, 3e38);
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 1.);
        let pose = at(&world, a).position;
        assert!(pose.is_finite() && (0. ..=2.).contains(&pose.x), "{pose:?}");
        // Its time still a time: at ordinary speeds it plays on.
        world.play(animator, &clip, Play::new()).unwrap();
        world.set_animator_speed(animator, 1.);
        let before = at(&world, a).position.x;
        clock.frame(&world, 1.1);
        let after = at(&world, a).position.x;
        assert!(
            close((after - before).rem_euclid(2.), 0.2),
            "{before} to {after}"
        );
    }

    #[test]
    fn a_mask_weight_not_a_number_takes_none() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.play(animator, &slide(1., 1.), Play::new()).unwrap();
        world
            .play(animator, &slide(3., 3.), Play::new().layer(1))
            .unwrap();
        let mask = Mask::new().node(a, f32::NAN);
        world
            .set_layer(animator, 1, Layer::new().mask(mask))
            .unwrap();
        Clock(Instant::now()).frame(&world, 0.);
        assert_eq!(at(&world, a).position.x, 1., "the layer under it");
    }

    /// A chain straight up from the origin: `a` there, `b` 1 above, `c` 1
    /// above that, placed as a model's.
    fn arm(world: &World) -> (Placed, [ObjectId; 3]) {
        let root = world.add(Object::group());
        let a = world.add(Object::group().child_of(root));
        let b = world.add(Object::group().at([0., 1., 0.]).child_of(a));
        let c = world.add(Object::group().at([0., 1., 0.]).child_of(b));
        let placed = Placed {
            root,
            nodes: vec![Some(a), Some(b), Some(c)],
            meshes: vec![Vec::new(); 3],
            lights: Vec::new(),
            cameras: Vec::new(),
        };
        (placed, [a, b, c])
    }
    fn place(world: &World, id: ObjectId) -> Vec3 {
        let [x, y, z, _] = world.world_matrix(id).unwrap().columns[3];
        vec3(x, y, z)
    }
    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }
    /// The arm with an IK hook made from its chain, posed once.
    fn bent(
        hook: impl FnOnce([ObjectId; 3]) -> Box<dyn FnMut(&mut crate::ik::Pose<'_>)>,
    ) -> (World, crate::AnimatorId, [ObjectId; 3], Clock) {
        let world = World::new();
        let (placed, chain) = arm(&world);
        let animator = world.add_animator(&placed);
        world.set_ik(animator, hook(chain));
        world.repose(animator);
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        (world, animator, chain, clock)
    }

    #[test]
    fn two_bone_reaches_its_target_bending_toward_its_pole() {
        let poles = [
            (vec3(5., 0., 0.), vec3(1., 0., 0.)),
            (vec3(0., 5., 0.), vec3(0., 1., 0.)),
        ];
        for (pole, middle) in poles {
            let (world, _, [_, b, c], _) = bent(|chain| {
                Box::new(move |pose| {
                    let target = vec3(1., 1., 0.);
                    assert!(crate::ik::two_bone(pose, chain, target, Some(pole), 1.));
                })
            });
            assert!(
                near(place(&world, c), vec3(1., 1., 0.)),
                "{:?}",
                place(&world, c)
            );
            assert!(
                near(place(&world, b), middle),
                "pole {pole:?}: {:?}",
                place(&world, b)
            );
        }
    }

    #[test]
    fn two_bone_stretches_toward_a_target_out_of_reach_and_minds_its_inputs() {
        let target = std::rc::Rc::new(std::cell::Cell::new(vec3(3., 0., 0.)));
        let weight = std::rc::Rc::new(std::cell::Cell::new(1.));
        let (t, w) = (target.clone(), weight.clone());
        let (world, animator, [_, b, c], clock) = bent(move |chain| {
            Box::new(move |pose| {
                crate::ik::two_bone(pose, chain, t.get(), None, w.get());
            })
        });
        assert!(
            near(place(&world, c), vec3(2., 0., 0.)),
            "{:?}",
            place(&world, c)
        );
        // Not quite straight (it reaches all but a ten-thousandth).
        assert!((place(&world, b) - vec3(1., 0., 0.)).length() < 0.02);
        // No weight, or an input not a number: the pose as the layers made
        // it (the chain straight up, at rest).
        let inputs = [
            (vec3(1., 1., 0.), 0.),
            (vec3(f32::NAN, 0., 0.), 1.),
            (vec3(1., 1., 0.), f32::NAN),
        ];
        for (to, by) in inputs {
            target.set(to);
            weight.set(by);
            world.repose(animator);
            clock.frame(&world, 0.1);
            let end = place(&world, c);
            assert!(near(end, vec3(0., 2., 0.)), "{to:?} {by}: {end:?}");
        }
    }

    #[test]
    fn look_at_turns_a_node_toward_its_target_as_far_as_allowed() {
        for (limit, turned) in [(3., std::f32::consts::FRAC_PI_2), (0.5, 0.5)] {
            let world = World::new();
            let (placed, [a, _, _]) = arm(&world);
            let animator = world.add_animator(&placed);
            world.set_ik(animator, move |pose| {
                let forward = vec3(0., 0., 1.);
                assert!(crate::ik::look_at(
                    pose,
                    a,
                    forward,
                    vec3(5., 0., 0.),
                    1.,
                    limit
                ));
            });
            world.repose(animator);
            Clock(Instant::now()).frame(&world, 0.);
            assert!(
                close(angle(at(&world, a).rotation), turned),
                "{:?}",
                at(&world, a)
            );
        }
    }

    #[test]
    fn an_ik_hook_bends_the_blended_pose_each_frame() {
        let world = World::new();
        let (placed, joints) = arm(&world);
        let [a, _, c] = joints;
        let animator = world.add_animator(&placed);
        // A clip swaying the upper joint; IK keeps the end on its target.
        let sway = clip(vec![(
            0,
            Path::Rotation,
            vec![0., 1.],
            vec![turn(0.), turn(1.)],
        )]);
        world.play(animator, &sway, Play::new()).unwrap();
        let target = std::rc::Rc::new(std::cell::Cell::new(vec3(1., 1., 0.)));
        let t = target.clone();
        world.set_ik(animator, move |pose| {
            crate::ik::two_bone(pose, joints, t.get(), Some(vec3(0., 5., 0.)), 1.);
        });
        let clock = Clock(Instant::now());
        frames(&clock, &world, 0., 1.);
        assert!(near(place(&world, c), vec3(1., 1., 0.)));
        assert!(
            !close(angle(at(&world, a).rotation), 0.),
            "the clip still turns it"
        );
        // Nothing playing: a moved target waits for `repose`.
        world.stop(animator, 0, 0.);
        clock.frame(&world, 1.25);
        target.set(vec3(0., 1., 1.));
        clock.frame(&world, 1.5);
        assert!(near(place(&world, c), vec3(1., 1., 0.)), "not posed again");
        world.repose(animator);
        clock.frame(&world, 1.75);
        assert!(
            near(place(&world, c), vec3(0., 1., 1.)),
            "{:?}",
            place(&world, c)
        );
        assert!(!world.view().animating());
    }

    #[test]
    fn two_bone_reaches_any_target_within_reach_from_any_pose() {
        // A fixed sequence, the same every run.
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2. - 1.
        };
        for case in 0..200 {
            let world = World::new();
            let (placed, chain) = arm(&world);
            let [a, b, c] = chain;
            let mut rotation = || Quat::from_euler(next() * 3., next() * 3., next() * 3.);
            let (turned, bent_a, bent_b) = (rotation(), rotation(), rotation());
            world.update(placed.root, |o| o.transform.rotation = turned);
            world.update(a, |o| o.transform.rotation = bent_a);
            world.update(b, |o| o.transform.rotation = bent_b);
            let direction = vec3(next(), next(), next()).normalized();
            let target = direction * (0.2 + (next() + 1.) * 0.85);
            let pole = vec3(next(), next(), next()) * 4.;
            let animator = world.add_animator(&placed);
            world.set_ik(animator, move |pose| {
                crate::ik::two_bone(pose, chain, target, Some(pole), 1.);
            });
            world.repose(animator);
            Clock(Instant::now()).frame(&world, 0.);
            let (end, middle) = (place(&world, c), place(&world, b));
            assert!(near(end, target), "case {case}: {end:?} for {target:?}");
            // The middle joint on the pole's side of the line to the target.
            let axis = target.normalized();
            let flat = |p: Vec3| p - axis * p.dot(axis);
            let side = flat(middle).dot(flat(pole));
            assert!(
                side >= -1e-4,
                "case {case}: middle {middle:?} pole {pole:?}"
            );
        }
    }

    #[test]
    fn look_at_faces_any_target_from_any_pose() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2. - 1.
        };
        for case in 0..200 {
            let world = World::new();
            let (placed, [a, b, _]) = arm(&world);
            let mut rotation = || Quat::from_euler(next() * 3., next() * 3., next() * 3.);
            let (turned, bent) = (rotation(), rotation());
            world.update(a, |o| o.transform.rotation = turned);
            world.update(b, |o| o.transform.rotation = bent);
            let target = vec3(next(), next(), next()) * 5.;
            let animator = world.add_animator(&placed);
            world.set_ik(animator, move |pose| {
                crate::ik::look_at(pose, b, vec3(0., 0., 1.), target, 1., std::f32::consts::PI);
            });
            world.repose(animator);
            Clock(Instant::now()).frame(&world, 0.);
            let m = world.world_matrix(b).unwrap();
            let facing = vec3(m.columns[2][0], m.columns[2][1], m.columns[2][2]).normalized();
            let wanted = (target - place(&world, b)).normalized();
            assert!(
                facing.dot(wanted) > 0.9999,
                "case {case}: {facing:?} {wanted:?}"
            );
        }
    }

    #[test]
    fn ik_sees_the_hierarchy_as_it_is_not_as_it_was() {
        // Set before the hierarchy changed, or after: the same frame.
        let run = |ik_first: bool, change: &dyn Fn(&World, &Placed, [ObjectId; 3])| {
            let world = World::new();
            let (placed, chain) = arm(&world);
            let animator = world.add_animator(&placed);
            let set = |world: &World| {
                world.set_ik(animator, move |pose| {
                    crate::ik::two_bone(pose, chain, vec3(1., 1., 0.), Some(vec3(5., 0., 0.)), 1.);
                });
            };
            if ik_first {
                set(&world);
                change(&world, &placed, chain);
            } else {
                change(&world, &placed, chain);
                set(&world);
            }
            world.repose(animator);
            Clock(Instant::now()).frame(&world, 0.);
            (place(&world, chain[1]), place(&world, chain[2]))
        };
        // The model carried on a vehicle; then its middle joint moved out
        // from under its upper (no chain left to bend).
        let vehicle = |world: &World, placed: &Placed, _: [ObjectId; 3]| {
            let car = world.add(Object::group().at([5., 0., 0.]));
            world.update(car, |o| o.transform.rotation = Quat::from_euler(0., 1., 0.));
            world.update(placed.root, |o| o.parent = Some(car));
        };
        let broken = |world: &World, placed: &Placed, [_, b, _]: [ObjectId; 3]| {
            world.update(b, |o| o.parent = Some(placed.root));
        };
        for change in [&vehicle as &dyn Fn(&World, &Placed, [ObjectId; 3]), &broken] {
            let (retained, fresh) = (run(true, change), run(false, change));
            assert!(
                near(retained.0, fresh.0) && near(retained.1, fresh.1),
                "{retained:?} {fresh:?}"
            );
        }
    }

    #[test]
    fn a_mask_below_a_node_follows_the_hierarchy() {
        let run = |mask_first: bool| {
            let world = World::new();
            let (placed, a, b) = rig(&world);
            let animator = world.add_animator(&placed);
            let both = clip(vec![
                (0, Path::Translation, vec![0.], vec![x(1.)]),
                (1, Path::Translation, vec![0.], vec![[1., 1., 0., 0.]]),
            ]);
            world.play(animator, &both, Play::new().layer(1)).unwrap();
            let set = |world: &World| {
                let below = Layer::new().mask(Mask::new().below(a, 1.));
                world.set_layer(animator, 1, below).unwrap();
            };
            // `b` taken from under `a`: the mask below `a` no longer has it.
            let moved = |world: &World| world.update(b, |o| o.parent = Some(placed.root));
            if mask_first {
                set(&world);
                moved(&world);
            } else {
                moved(&world);
                set(&world);
            }
            Clock(Instant::now()).frame(&world, 0.);
            (at(&world, a).position, at(&world, b).position)
        };
        let (retained, fresh) = (run(true), run(false));
        assert_eq!(retained, fresh);
        assert_eq!(fresh.1, vec3(0., 1., 0.), "b at rest");
    }

    #[test]
    fn root_motion_is_measured_as_the_model_is_turned_now() {
        let run = |root_first: bool| {
            let world = World::new();
            let (placed, a, b) = rig(&world);
            let animator = world.add_animator(&placed);
            let walk = clip(vec![(
                1,
                Path::Translation,
                vec![0., 1.],
                vec![[0., 1., 0., 0.], [2., 1., 0., 0.]],
            )]);
            world.play(animator, &walk, Play::new()).unwrap();
            let quarter = std::f32::consts::FRAC_PI_2;
            let turn = |world: &World| {
                world.update(a, |o| {
                    o.transform.rotation = Quat::from_axis_angle(Vec3::Y, quarter)
                })
            };
            let set = |world: &World| world.set_root_motion(animator, Some(RootMotion::new(b)));
            if root_first {
                set(&world);
                turn(&world);
            } else {
                turn(&world);
                set(&world);
            }
            let clock = Clock(Instant::now());
            frames(&clock, &world, 0., 0.5);
            world.animator_update(animator).moved
        };
        let (retained, fresh) = (run(true), run(false));
        assert!(near(retained, fresh), "{retained:?} {fresh:?}");
        assert!(
            near(fresh, vec3(0., 0., -1.)),
            "x turned a quarter about y: -z; {fresh:?}"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "an IK hook called its world")]
    fn an_ik_hook_calling_its_world_is_told_why_it_cannot() {
        let world = World::new();
        let (placed, [a, _, _]) = arm(&world);
        let animator = world.add_animator(&placed);
        let held = world.clone();
        world.set_ik(animator, move |_| {
            held.update(a, |o| o.transform.position = vec3(1., 0., 0.));
        });
        world.repose(animator);
        Clock(Instant::now()).frame(&world, 0.);
    }

    /// Node `a` holding a mesh of two morph targets (weights 0 and 0.7 at
    /// rest), placed as a model's.
    fn morphing(world: &World) -> (Placed, ObjectId) {
        let root = world.add(Object::group());
        let a = world.add(Object::group().child_of(root));
        let shape = crate::Geometry::cuboid(1., 1., 1.);
        let up = vec![[0., 0.5, 0.]; shape.vertices().len()];
        let geometry = shape
            .with_morph_targets(vec![
                crate::MorphTarget::new(up.clone()),
                crate::MorphTarget::new(up),
            ])
            .unwrap();
        let mesh = world.add(Object::mesh(geometry, crate::Material::default()).child_of(a));
        world.set_morph_weights(mesh, &[0., 0.7]);
        let placed = Placed {
            root,
            nodes: vec![Some(a)],
            meshes: vec![vec![mesh]],
            lights: Vec::new(),
            cameras: Vec::new(),
        };
        (placed, mesh)
    }
    /// Target 0's weight going `from` to `to` over a second.
    fn morph(from: f32, to: f32) -> Clip {
        clip(vec![(
            0,
            Path::Weight(0),
            vec![0., 1.],
            vec![x(from), x(to)],
        )])
    }
    fn weights(world: &World, mesh: ObjectId) -> Vec<f32> {
        let inner = world.inner();
        inner
            .morph_weights
            .get(&mesh.index)
            .map(|(_, w)| w.clone())
            .unwrap_or_default()
    }

    #[test]
    fn morph_weights_play_as_placed_pose_plays_them() {
        let world = World::new();
        let (placed, mesh) = morphing(&world);
        let animator = world.add_animator(&placed);
        let clip = morph(0., 1.);
        world.play(animator, &clip, Play::new()).unwrap();
        let other = World::new();
        let (theirs, their_mesh) = morphing(&other);
        let clock = Clock(Instant::now());
        for t in [0., 0.25, 0.5, 0.75] {
            clock.frame(&world, t);
            theirs.pose(&other, &clip, t);
            assert_eq!(weights(&world, mesh), weights(&other, their_mesh), "at {t}");
        }
        assert_eq!(
            weights(&world, mesh),
            [0.75, 0.7],
            "target 1 keeps its rest"
        );
    }

    #[test]
    fn morph_weights_crossfade_and_add_and_follow_masks() {
        let world = World::new();
        let (placed, mesh) = morphing(&world);
        let a = placed.node(0).unwrap();
        let animator = world.add_animator(&placed);
        world.play(animator, &morph(0.2, 0.2), Play::new()).unwrap();
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        world
            .play(animator, &morph(0.6, 0.6), Play::new().fade(1.))
            .unwrap();
        clock.frame(&world, 0.1);
        clock.frame(&world, 0.35);
        clock.frame(&world, 0.6);
        assert!(
            close(weights(&world, mesh)[0], 0.4),
            "halfway: {:?}",
            weights(&world, mesh)
        );
        clock.frame(&world, 0.85);
        clock.frame(&world, 1.1);
        assert!(close(weights(&world, mesh)[0], 0.6));
        // Half of a clip going 0 to 0.5 added, at its end: 0.6 + 0.25.
        world
            .set_layer(animator, 1, Layer::additive().weight(0.5))
            .unwrap();
        world
            .play(animator, &morph(0., 0.5), Play::new().layer(1).once())
            .unwrap();
        frames(&clock, &world, 1.25, 2.5);
        assert!(
            close(weights(&world, mesh)[0], 0.85),
            "{:?}",
            weights(&world, mesh)
        );
        // Masked off its node: no more.
        let elsewhere = Layer::additive().weight(0.5).mask(Mask::new().node(a, 0.));
        world.set_layer(animator, 1, elsewhere).unwrap();
        clock.frame(&world, 2.75);
        assert!(close(weights(&world, mesh)[0], 0.6));
        // Held: nothing written again.
        let stamp = world.inner().stamps[mesh.index as usize];
        clock.frame(&world, 3.);
        assert_eq!(world.inner().stamps[mesh.index as usize], stamp);
    }
}
