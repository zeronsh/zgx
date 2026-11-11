//! Inverse kinematics (feature `gltf`): an animator's pose bent after its
//! layers blend it and before it's drawn (`World::set_ik`), with two
//! solvers supplied: `two_bone` (an arm, a leg) and `look_at` (a head, an
//! eye, a turret). Rotations compose as if scales are uniform.
use crate::{Mat4, ObjectId, Quat, Transform, Vec3, vec3};

/// An animator's pose as its layers made it this frame, for an IK hook to
/// read and bend (`World::set_ik`): each node's place in its parent's
/// frame, and where that puts it in the world.
pub struct Pose<'a> {
    /// The model's nodes, by index.
    pub(crate) nodes: &'a [Option<ObjectId>],
    /// Each node's parent among them.
    pub(crate) parents: &'a [Option<usize>],
    /// The world's matrix above each node whose parent isn't the model's
    /// (`None` hidden, or under the model).
    pub(crate) outer: &'a [Option<Mat4>],
    /// The pose, by node index.
    pub(crate) locals: &'a mut [Transform],
    /// The nodes the animator writes; a node set here joins them.
    pub(crate) posed: &'a mut Vec<u32>,
}
impl Pose<'_> {
    fn index(&self, node: ObjectId) -> Option<usize> {
        self.nodes.iter().position(|n| *n == Some(node))
    }
    /// `node`'s place in its parent's frame, as posed so far; `None` for
    /// an object not of this model.
    pub fn local(&self, node: ObjectId) -> Option<Transform> {
        Some(self.locals[self.index(node)?])
    }
    /// Place `node` in its parent's frame (ignored for an object not of
    /// this model, or a transform not all numbers).
    pub fn set_local(&mut self, node: ObjectId, transform: Transform) {
        let Some(index) = self.index(node) else {
            return;
        };
        if !finite(&transform) {
            return;
        }
        self.locals[index] = transform;
        if let Err(at) = self.posed.binary_search(&(index as u32)) {
            self.posed.insert(at, index as u32);
        }
    }
    /// `node`'s matrix in the world as posed so far; `None` for one not of
    /// this model, hidden, or under a hidden one.
    pub fn world(&self, node: ObjectId) -> Option<Mat4> {
        self.world_of(self.index(node)?)
    }
    fn world_of(&self, index: usize) -> Option<Mat4> {
        let mut matrix = self.locals[index].matrix();
        let mut at = index;
        for _ in 0..64 {
            match self.parents[at] {
                Some(parent) => {
                    matrix = self.locals[parent].matrix() * matrix;
                    at = parent;
                }
                None => return Some(self.outer[at]? * matrix),
            }
        }
        None
    }
    /// `node`'s parent's matrix in the world.
    fn parent_world(&self, index: usize) -> Option<Mat4> {
        match self.parents[index] {
            Some(parent) => self.world_of(parent),
            None => self.outer[index],
        }
    }
}

fn finite(t: &Transform) -> bool {
    let r = t.rotation;
    t.position.is_finite()
        && t.scale.is_finite()
        && [r.x, r.y, r.z, r.w].iter().all(|v| v.is_finite())
}

/// Bend a chain of three nodes (`upper` the parent of `middle`, the
/// parent of `end`: a shoulder, an elbow, a wrist) so `end` reaches
/// `target` (in the world), or as near as the chain stretches. The middle
/// joint bends toward `pole` (a point in the world: an elbow's out, a
/// knee's ahead) if given, else as it bent already. `weight` (0 to 1) is
/// how much of the way from the pose to the solution. Whether it changed
/// the pose: not for nodes not chained so, not of this model, or inputs
/// not numbers.
pub fn two_bone(
    pose: &mut Pose<'_>,
    [upper, middle, end]: [ObjectId; 3],
    target: Vec3,
    pole: Option<Vec3>,
    weight: f32,
) -> bool {
    let weight = if weight.is_nan() {
        0.
    } else {
        weight.clamp(0., 1.)
    };
    let pole = pole.filter(|p| p.is_finite());
    let (Some(a), Some(b), Some(c)) = (pose.index(upper), pose.index(middle), pose.index(end))
    else {
        return false;
    };
    if weight == 0.
        || !target.is_finite()
        || pose.parents[b] != Some(a)
        || pose.parents[c] != Some(b)
    {
        return false;
    }
    let (Some(above), Some(wa), Some(wb), Some(wc)) = (
        pose.parent_world(a),
        pose.world_of(a),
        pose.world_of(b),
        pose.world_of(c),
    ) else {
        return false;
    };
    let (pa, pb, pc) = (origin(&wa), origin(&wb), origin(&wc));
    let (ab, bc) = ((pb - pa).length(), (pc - pb).length());
    if ab < 1e-6 || bc < 1e-6 {
        return false;
    }
    let reach = (target - pa).length().clamp(1e-4, (ab + bc) * 0.9999);
    // The angles at the upper and middle joints now, and those that put
    // the end `reach` from the upper.
    let angle = |u: Vec3, v: Vec3| {
        let (u, v) = (u.normalized(), v.normalized());
        u.dot(v).clamp(-1., 1.).acos()
    };
    let cosine = |x: f32| x.clamp(-1., 1.).acos();
    let at_upper = angle(pc - pa, pb - pa);
    let at_middle = angle(pa - pb, pc - pb);
    let upper_to = cosine((bc * bc - ab * ab - reach * reach) / (-2. * ab * reach));
    let middle_to = cosine((reach * reach - ab * ab - bc * bc) / (-2. * ab * bc));
    // The plane it bends in: as it bends now; straight, any square to it.
    let mut bend = (pc - pa).cross(pb - pa);
    if bend.length() < 1e-6 {
        let along = (pc - pa).normalized();
        let other = if along.x.abs() < 0.9 {
            vec3(1., 0., 0.)
        } else {
            Vec3::Y
        };
        bend = along.cross(other);
    }
    let bend = bend.normalized();
    let mut swing = (pc - pa).cross(target - pa);
    let toward = angle(pc - pa, target - pa);
    if swing.length() < 1e-6 {
        swing = bend;
    }
    let swing = swing.normalized();
    let (ga, gb) = (rotation(&wa), rotation(&wb));
    let local_axis = |g: Quat, axis: Vec3| conjugate(g).rotate(axis);
    let r0 = Quat::from_axis_angle(local_axis(ga, bend), upper_to - at_upper);
    let r1 = Quat::from_axis_angle(local_axis(gb, bend), middle_to - at_middle);
    let r2 = Quat::from_axis_angle(local_axis(ga, swing), toward);
    let (la, lb) = (pose.locals[a], pose.locals[b]);
    let mut upper_rotation = (la.rotation * (r2 * r0)).normalized();
    let middle_rotation = (lb.rotation * r1).normalized();
    // Turned about the line to the target, so the middle joint is on the
    // pole's side.
    if let Some(pole) = pole {
        let placed = |rotation: Quat| {
            let upper = above * Transform { rotation, ..la }.matrix();
            let middle = upper
                * Transform {
                    rotation: middle_rotation,
                    ..lb
                }
                .matrix();
            origin(&middle)
        };
        let axis = (target - pa).normalized();
        let flat = |p: Vec3| {
            let d = p - pa;
            d - axis * d.dot(axis)
        };
        let (now, want) = (flat(placed(upper_rotation)), flat(pole));
        if now.length() > 1e-6 && want.length() > 1e-6 {
            let turn = angle(now, want) * now.cross(want).dot(axis).signum();
            // A turn about a world line through the upper joint, in its
            // frame before it's bent (`ga`), applied before the bend: the
            // whole solved chain turns about it, its end staying put.
            let turn = Quat::from_axis_angle(local_axis(ga, axis), turn);
            upper_rotation = (la.rotation * turn * (r2 * r0)).normalized();
        }
    }
    let blend = |from: Quat, to: Quat| nlerp(from, to, weight);
    pose.set_local(
        upper,
        Transform {
            rotation: blend(la.rotation, upper_rotation),
            ..la
        },
    );
    pose.set_local(
        middle,
        Transform {
            rotation: blend(lb.rotation, middle_rotation),
            ..lb
        },
    );
    true
}

/// Turn `node` so its own `forward` axis (in its frame: `vec3(0., 0., 1.)`
/// for glTF's +z) points at `target` (in the world), by at most
/// `max_angle` radians from its pose and `weight` (0 to 1) of the way.
/// Whether it changed the pose: not for a node not of this model, at the
/// target, or inputs not numbers.
pub fn look_at(
    pose: &mut Pose<'_>,
    node: ObjectId,
    forward: Vec3,
    target: Vec3,
    weight: f32,
    max_angle: f32,
) -> bool {
    let weight = if weight.is_nan() {
        0.
    } else {
        weight.clamp(0., 1.)
    };
    let max_angle = if max_angle.is_nan() {
        0.
    } else {
        max_angle.clamp(0., std::f32::consts::PI)
    };
    let Some(index) = pose.index(node) else {
        return false;
    };
    if weight == 0. || max_angle == 0. || !target.is_finite() || !forward.is_finite() {
        return false;
    }
    let Some(world) = pose.world_of(index) else {
        return false;
    };
    let global = rotation(&world);
    let facing = global.rotate(forward.normalized());
    let wanted = target - origin(&world);
    if wanted.length() < 1e-6 || facing.length() < 1e-6 {
        return false;
    }
    let wanted = wanted.normalized();
    let full = facing.dot(wanted).clamp(-1., 1.).acos();
    let mut axis = facing.cross(wanted);
    if axis.length() < 1e-6 {
        if full < 1e-6 {
            return false;
        }
        // Facing away: about any axis square to it.
        let other = if facing.x.abs() < 0.9 {
            vec3(1., 0., 0.)
        } else {
            Vec3::Y
        };
        axis = facing.cross(other);
    }
    let turn = full.min(max_angle) * weight;
    // Turned in the world, then back into its parent's frame.
    let world_turn = Quat::from_axis_angle(axis.normalized(), turn);
    let local = pose.locals[index];
    let rotation = (local.rotation * (conjugate(global) * world_turn * global)).normalized();
    pose.set_local(node, Transform { rotation, ..local });
    true
}

fn origin(m: &Mat4) -> Vec3 {
    let [x, y, z, _] = m.columns[3];
    vec3(x, y, z)
}
/// The rotation of `m`'s upper 3x3, its columns' scales taken out.
fn rotation(m: &Mat4) -> Quat {
    let c = &m.columns;
    let axis = |i: usize| vec3(c[i][0], c[i][1], c[i][2]).normalized();
    let (x, y, z) = (axis(0), axis(1), axis(2));
    // Shepperd's method, from the rotation matrix's columns.
    let (m00, m11, m22) = (x.x, y.y, z.z);
    let trace = m00 + m11 + m22;
    let q = if trace > 0. {
        let s = (trace + 1.).sqrt() * 2.;
        Quat {
            w: 0.25 * s,
            x: (y.z - z.y) / s,
            y: (z.x - x.z) / s,
            z: (x.y - y.x) / s,
        }
    } else if m00 > m11 && m00 > m22 {
        let s = (1. + m00 - m11 - m22).sqrt() * 2.;
        Quat {
            w: (y.z - z.y) / s,
            x: 0.25 * s,
            y: (y.x + x.y) / s,
            z: (z.x + x.z) / s,
        }
    } else if m11 > m22 {
        let s = (1. + m11 - m00 - m22).sqrt() * 2.;
        Quat {
            w: (z.x - x.z) / s,
            x: (y.x + x.y) / s,
            y: 0.25 * s,
            z: (z.y + y.z) / s,
        }
    } else {
        let s = (1. + m22 - m00 - m11).sqrt() * 2.;
        Quat {
            w: (x.y - y.x) / s,
            x: (z.x + x.z) / s,
            y: (z.y + y.z) / s,
            z: 0.25 * s,
        }
    };
    q.normalized()
}
fn conjugate(q: Quat) -> Quat {
    Quat {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
    }
}
/// `t` of the way from `a` to `b`, the shorter way, at length 1.
fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    if t >= 1. {
        return b;
    }
    let dot = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    let s = if dot < 0. { -t } else { t };
    Quat {
        x: a.x * (1. - t) + b.x * s,
        y: a.y * (1. - t) + b.y * s,
        z: a.z * (1. - t) + b.z * s,
        w: a.w * (1. - t) + b.w * s,
    }
    .normalized()
}
