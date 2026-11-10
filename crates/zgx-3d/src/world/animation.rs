//! A world's animators (`World::add_animator`): played by its frames' time.
use super::{Inner, Settle, Slot, World, claim, release};
use crate::{
    Clip, Mat4, Placed, Quat, Transform,
    animator::{Animator, AnimatorId, AnimatorUpdate, Layer, MAX_STEP, Play},
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
            animator.step(dt);
            animator.evaluate();
            changed |= write(inner, animator);
        }
        playing |= animator.playing();
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
    changed
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
        let animator = Animator::new(placed.nodes.clone(), rest);
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
    pub fn play(&self, id: AnimatorId, clip: &Clip, play: Play) {
        self.change_animator(id, |animator| animator.play(clip, play));
    }
    /// Fade out what animator `id` plays on `layer` over `fade` seconds
    /// (at once if 0): the layers under it show through.
    pub fn stop(&self, id: AnimatorId, layer: usize, fade: f32) {
        let fade = if fade.is_finite() { fade.max(0.) } else { 0. };
        self.change_animator(id, |animator| animator.stop(layer, fade));
    }
    /// Animator `id`'s layer `index` (counted from 0, the lowest) as
    /// `layer` says: its weight, mask and blend. Layers up to it that
    /// weren't set override wholly.
    pub fn set_layer(&self, id: AnimatorId, index: usize, layer: Layer) {
        let mask = {
            let inner = self.inner.borrow();
            let Some(animator) = Self::animator_ref(&inner, id) else {
                return;
            };
            layer.mask.as_ref().map(|mask| {
                let nodes = &animator.nodes;
                let at: std::collections::HashMap<_, _> = nodes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, node)| Some((((*node)?), i)))
                    .collect();
                let parent = |i: usize| {
                    let id = nodes[i]?;
                    let index = inner.live(id.index, id.generation)?;
                    let parent = inner.objects[index].value.as_ref()?.parent?;
                    at.get(&parent).copied()
                };
                mask.resolve(nodes, parent)
            })
        };
        self.change_animator(id, |animator| animator.set_layer(index, layer, mask));
    }
    /// Animator `id`'s speed, on its clips' own: 1 by default, 0 pauses it
    /// (and the world settles), negative plays them backwards.
    pub fn set_animator_speed(&self, id: AnimatorId, speed: f32) {
        let speed = if speed.is_finite() { speed } else { 1. };
        self.change_animator(id, |animator| animator.speed = speed);
    }
    /// What animator `id` did since this was last asked: its clips' events
    /// passed. Empty for one gone.
    pub fn animator_update(&self, id: AnimatorId) -> AnimatorUpdate {
        let mut inner = self.inner.borrow_mut();
        Self::animator_of(&mut inner.animators.slots, id)
            .map(|animator| std::mem::take(&mut animator.update))
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
    fn change_animator(&self, id: AnimatorId, f: impl FnOnce(&mut Animator)) {
        let mut inner = self.inner.borrow_mut();
        let Some(animator) = Self::animator_of(&mut inner.animators.slots, id) else {
            return;
        };
        f(animator);
        let playing = Self::any_playing(&inner);
        inner.unsettle(Settle::ANIMATION, playing);
        drop(inner);
        // A frame poses it.
        self.view.invalidate();
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
        Clip, Layer, Mask, Object, ObjectId, Placed, Play, Quat, Transform, Vec3, World,
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
        world.play(animator, &slide(0., 2.), Play::new());
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
        world.play(animator, &clip, Play::new());
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
        world.play(animator, &slide(0., 0.), Play::new());
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        world.play(animator, &slide(2., 2.), Play::new().fade(1.));
        clock.frame(&world, 0.1);
        assert_eq!(at(&world, a).position.x, 0., "the new clip's first frame");
        clock.frame(&world, 0.35);
        assert!(close(at(&world, a).position.x, 0.5), "{:?}", at(&world, a));
        clock.frame(&world, 0.6);
        assert!(close(at(&world, a).position.x, 1.), "halfway");
        clock.frame(&world, 0.85);
        clock.frame(&world, 1.1);
        assert_eq!(at(&world, a).position.x, 2., "done: the new clip alone");
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
        world.play(animator, &both(1.), Play::new());
        world.play(animator, &both(3.), Play::new().layer(1));
        world.set_layer(animator, 1, Layer::new().weight(0.25));
        let clock = Clock(Instant::now());
        clock.frame(&world, 0.);
        assert_eq!(at(&world, a).position.x, 1.5, "a quarter of the way");
        assert_eq!(at(&world, b).position.x, 1.5);
        // Masked to `b` alone, at half.
        world.set_layer(animator, 1, Layer::new().mask(Mask::new().node(b, 0.5)));
        clock.frame(&world, 0.1);
        assert_eq!(at(&world, a).position.x, 1., "unmasked: under it");
        assert_eq!(at(&world, b).position.x, 2.);
        // Masked below `a`: both.
        world.set_layer(animator, 1, Layer::new().mask(Mask::new().below(a, 1.)));
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
        world.play(animator, &base, Play::new());
        world.play(animator, &lean, Play::new().layer(1).once());
        world.set_layer(animator, 1, Layer::additive().weight(0.5));
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
        world.play(animator, &slide(0., 2.), Play::new().once());
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
        world.play(animator, &slide(0., 1.), Play::new());
        assert!(world.view().animating());
        world.set_animator_speed(animator, 0.);
        assert!(!world.view().animating());
        world.set_animator_speed(animator, 1.);
        assert!(world.view().animating());
        world.remove_animator(animator);
        assert!(!world.view().animating());
        world.play(animator, &slide(0., 1.), Play::new());
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
            world.play(animator, &clip, Play::new().speed(1.3));
            let clock = Clock(Instant::now());
            let mut poses = Vec::new();
            for (i, &t) in times.iter().enumerate() {
                if i == 4 {
                    world.play(animator, &slide(1., 2.), Play::new().fade(0.3));
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

    #[test]
    fn a_long_gap_between_frames_is_taken_as_a_short_step() {
        let world = World::new();
        let (placed, a, _) = rig(&world);
        let animator = world.add_animator(&placed);
        world.play(animator, &slide(0., 4.), Play::new().once());
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
        world.play(animator, &clip, Play::new());
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
}
