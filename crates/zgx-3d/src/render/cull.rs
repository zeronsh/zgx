//! What the camera can see: a frame's frustum as six planes from its view
//! and projection, and whether a mesh's box in the world is outside it. A
//! mesh outside draws nothing in the meshes' pass (nor in what is behind
//! glass); one partly in, or near an edge, stays: the test never removes
//! what shows. Shadows still draw every caster (culling casters against the
//! sun's volume comes with cascades). The meshes left draw in the order
//! they had, each run's in spans of its instances, so a frame shows what it
//! would with nothing culled.
use super::{Run, Scratch, shadows::box_through};
use crate::math::{Mat4, Vec3};

/// Each instance's mesh's box in the world, by axis (lows' x, y, z, then
/// highs'): a frustum tests them a plane at a time, in loops of plain
/// arithmetic. `sure` says the box holds what draws: a posed mesh drawn in
/// its bind pose (a device without storage buffers in vertex shaders) isn't
/// culled.
#[derive(Default)]
pub(super) struct Boxes {
    axes: [Vec<f32>; 6],
    sure: Vec<bool>,
}
impl Boxes {
    pub(super) fn len(&self) -> usize {
        self.sure.len()
    }
    /// Whether instance `at`'s box holds what draws.
    #[cfg(feature = "shaders")]
    pub(super) fn sure(&self, at: usize) -> bool {
        self.sure[at]
    }
    /// As many as `len` (new ones empty and not sure, to be `set`).
    pub(super) fn resize(&mut self, len: usize) {
        self.axes.iter_mut().for_each(|axis| axis.resize(len, 0.));
        self.sure.resize(len, false);
    }
    /// Instance `at`'s box (`at < len()`; the frame writes them in step,
    /// `update`, and a custom material moving its vertices grows its own,
    /// `custom::grow_boxes`).
    #[cfg_attr(not(any(test, feature = "shaders")), allow(dead_code))]
    pub(super) fn set(&mut self, at: usize, (low, high): (Vec3, Vec3), sure: bool) {
        let values = [low.x, low.y, low.z, high.x, high.y, high.z];
        for (axis, value) in self.axes.iter_mut().zip(values) {
            axis[at] = value;
        }
        self.sure[at] = sure;
    }
    /// The box holding those of the instances in `ranges`.
    pub(super) fn union(
        &self,
        ranges: impl Iterator<Item = std::ops::Range<usize>>,
    ) -> (Vec3, Vec3) {
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for range in ranges {
            for i in 0..3 {
                for &v in &self.axes[i][range.clone()] {
                    low[i] = low[i].min(v);
                }
                for &v in &self.axes[3 + i][range.clone()] {
                    high[i] = high[i].max(v);
                }
            }
        }
        (Vec3::from(low), Vec3::from(high))
    }
}

/// `d`'s box in the world, as the sun's view fits it (`sun_view`): its
/// geometry's (morphed, if it is) through its matrix, or for a skinned mesh
/// through each of its joints' (about where its pose can take it).
#[inline(always)]
pub(super) fn world_box(d: &crate::world::Drawn) -> (Vec3, Vec3) {
    let bounds = match &d.morph {
        Some(active) => d.geometry.morphed_bounds(active),
        None => d.geometry.bounds(),
    };
    match d.palette {
        None => box_through(&d.matrix, bounds),
        Some(joints) => {
            let (mut low, mut high) = (Vec3::from([f32::MAX; 3]), Vec3::from([f32::MIN; 3]));
            for joint in joints {
                let (from, to) = box_through(joint, bounds);
                low = low.min(from);
                high = high.max(to);
            }
            (low, high)
        }
    }
}

/// The meshes' boxes for a frame that culls, `drawn` being its sorted list:
/// where the order is unmoved and the boxes are as of `since` (the last
/// frame making instances), those of meshes stamped since and posed ones;
/// else all. Then as of `meshes`. A posed mesh not drawn posed (`posing`
/// false: a device drawing it in its bind pose) isn't sure: not culled.
// Apart from the frame's code: inlined, it compiled the rest worse.
#[inline(never)]
pub(super) fn update(
    scratch: &mut Scratch,
    drawn: &[&crate::world::Drawn],
    (since, unmoved): (u64, bool),
    meshes: u64,
    posing: bool,
) {
    let boxes = &mut scratch.boxes;
    let current = unmoved && scratch.boxed == Some(since) && boxes.len() == drawn.len();
    boxes.resize(drawn.len());
    // Each axis's list beside the meshes, written in step: no index checked.
    let [lx, ly, lz, hx, hy, hz] = &mut boxes.axes;
    let axes = lx.iter_mut().zip(ly.iter_mut()).zip(lz.iter_mut());
    let axes = axes.zip(hx.iter_mut().zip(hy.iter_mut()).zip(hz.iter_mut()));
    for ((d, (((lx, ly), lz), ((hx, hy), hz))), sure) in
        drawn.iter().zip(axes).zip(boxes.sure.iter_mut())
    {
        // Posed in the world, its box follows its joints, which may be no
        // ancestors of it.
        let moving = d.palette.is_some() || d.morph.is_some();
        if !current || moving || d.stamp > since {
            let (low, high) = world_box(d);
            (*lx, *ly, *lz, *hx, *hy, *hz) = (low.x, low.y, low.z, high.x, high.y, high.z);
            *sure = posing || !moving;
        }
    }
    scratch.boxed = Some(meshes);
}

/// Which instances the camera may see, into `mask`: those whose boxes no
/// plane of `frustum` has wholly outside, and those not sure.
// Once a frame, out of its loop: inlined, the frame's code compiled worse.
#[inline(never)]
pub(super) fn visible(frustum: &Frustum, boxes: &Boxes, mask: &mut Vec<bool>) {
    mask.clear();
    mask.resize(boxes.len(), true);
    for &([a, b, c], d) in &frustum.planes {
        // The corner farthest along the plane's normal: each axis's high
        // where it points that way, low where not.
        let axis = |i: usize, n: f32| &boxes.axes[if n >= 0. { 3 + i } else { i }];
        let (x, y, z) = (axis(0, a), axis(1, b), axis(2, c));
        for (((seen, x), y), z) in mask.iter_mut().zip(x).zip(y).zip(z) {
            *seen &= a * x + b * y + c * z + d >= 0.;
        }
    }
    for (seen, sure) in mask.iter_mut().zip(&boxes.sure) {
        *seen |= !sure;
    }
}

/// Indices that cost the GPU about what a draw costs (the CPU recording it,
/// the GPU starting it): instances the camera can't see, between two it
/// can, are drawn with them where they hold fewer, rather than splitting
/// the run's draw. Drawn, they show nothing: a frame is the same. Measured
/// on an M5 Max (Metal, wall clock, interleaved): a draw more cost 142-147
/// ns, an off-screen instance's index 0.026-0.045 ns, so they cross at
/// 3,300-5,800 indices.
pub(super) const DRAW_INDICES: u32 = 4096;

/// Frames drawn without culling after one where it left out almost nothing
/// (`hardly`): its test and the spans' draws cost more than the few meshes
/// it would leave out; it looks again after these.
pub(super) const RESTING: u32 = 15;

/// Whether leaving out `culled` of `meshes` is almost nothing: under 2%.
pub(super) fn hardly(culled: u32, meshes: usize) -> bool {
    (culled as usize) * 50 < meshes
}

/// Each run's instances `mask` says the camera may see (with none, all), as
/// spans `(run, instances)` in the runs' order: one a run where none is
/// culled, a gap kept in its span where it holds fewer than `DRAW_INDICES`.
/// How many meshes were left out.
// Once a frame, out of its loop: inlined, the frame's code compiled worse.
#[inline(never)]
pub(super) fn shown(
    runs: &[Run],
    mask: Option<&[bool]>,
    out: &mut Vec<(u32, std::ops::Range<u32>)>,
    indices: impl Fn(&Run) -> u32,
) -> u32 {
    out.clear();
    let Some(mask) = mask else {
        out.extend((0..).zip(runs.iter().map(|run| run.instances.clone())));
        return 0;
    };
    let mut drawn = 0;
    for (r, run) in runs.iter().enumerate() {
        // The longest gap a span spans, asked for at the run's first gap
        // alone: its geometry is found by id.
        let mut gap = None;
        let mut span: Option<std::ops::Range<u32>> = None;
        for i in run.instances.clone() {
            if !mask[i as usize] {
                continue;
            }
            if let Some(open) = &mut span {
                let longest = || DRAW_INDICES / indices(run).max(1);
                if i == open.end || i - open.end <= *gap.get_or_insert_with(longest) {
                    open.end = i + 1;
                    continue;
                }
            }
            if let Some(done) = span.replace(i..i + 1) {
                drawn += done.len() as u32;
                out.push((r as u32, done));
            }
        }
        if let Some(done) = span {
            drawn += done.len() as u32;
            out.push((r as u32, done));
        }
    }
    runs.iter()
        .map(|run| run.instances.len() as u32)
        .sum::<u32>()
        - drawn
}

/// The six planes bounding what `view_projection` shows: `(normal, d)` with
/// what is inside where `normal · p + d >= 0`. Clip space's depth runs 0
/// (near) to 1 (far), as wgpu's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frustum {
    planes: [([f32; 3], f32); 6],
}
impl Frustum {
    pub(super) fn new(view_projection: &Mat4) -> Self {
        let c = &view_projection.columns;
        // Row `r` of the matrix (its columns are stored).
        let row = |r: usize| [c[0][r], c[1][r], c[2][r], c[3][r]];
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        let plane = |a: [f32; 4], sign: f32, b: [f32; 4]| {
            let p: [f32; 4] = std::array::from_fn(|i| a[i] + sign * b[i]);
            let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-12);
            ([p[0] / length, p[1] / length, p[2] / length], p[3] / length)
        };
        let zero = [0.; 4];
        Self {
            planes: [
                plane(w, 1., x),    // left: x >= -w
                plane(w, -1., x),   // right: x <= w
                plane(w, 1., y),    // bottom
                plane(w, -1., y),   // top
                plane(z, 1., zero), // near: z >= 0
                plane(w, -1., z),   // far: z <= w
            ],
        }
    }
    /// Whether the box `(low, high)` is wholly outside some plane: then
    /// nothing of it shows. (A box outside none, but outside the frustum
    /// past a corner, is kept: drawn, it shows nothing.)
    #[cfg(test)]
    pub(super) fn outside(&self, (low, high): (crate::Vec3, crate::Vec3)) -> bool {
        self.planes.iter().any(|&([a, b, c], d)| {
            // The box's corner farthest along the plane's normal.
            let far = |n: f32, l: f32, h: f32| if n >= 0. { h } else { l };
            a * far(a, low.x, high.x) + b * far(b, low.y, high.y) + c * far(c, low.z, high.z) + d
                < 0.
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Vec3, vec3};

    #[test]
    fn a_box_outside_the_frustum_is_culled_and_one_touching_it_is_not() {
        let view = Mat4::look_at(vec3(0., 0., 5.), Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective(1., 1., 0.1, 100.);
        let frustum = Frustum::new(&(projection * view));
        let at =
            |x: f32, y: f32, z: f32, r: f32| (vec3(x - r, y - r, z - r), vec3(x + r, y + r, z + r));
        assert!(!frustum.outside(at(0., 0., 0., 0.5)), "in front");
        assert!(frustum.outside(at(0., 0., 7., 0.5)), "behind the camera");
        assert!(frustum.outside(at(40., 0., 0., 0.5)), "far to the right");
        assert!(frustum.outside(at(0., -40., 0., 0.5)), "far below");
        assert!(
            frustum.outside(at(0., 0., -200., 0.5)),
            "past the far plane"
        );
        // Half a field of view is 0.5 rad: at 5 away the edge is 2.73 out.
        assert!(!frustum.outside(at(3.1, 0., 0., 0.5)), "across the edge");
        // (Its far face, 5.5 away, meets the edge at 3.0.)
        assert!(frustum.outside(at(3.6, 0., 0., 0.5)), "just past it");
        // Every point of a box that's kept may not show; none culled shows.
        assert!(
            !frustum.outside(at(0., 0., 4.95, 0.1)),
            "across the near plane"
        );
        // The plane-at-a-time test agrees, and keeps what isn't sure.
        let mut boxes = Boxes::default();
        let tests = [
            (0., 0., 0.),
            (0., 0., 7.),
            (40., 0., 0.),
            (3.1, 0., 0.),
            (3.6, 0., 0.),
        ];
        boxes.resize(tests.len() + 1);
        for (i, &(x, y, z)) in tests.iter().enumerate() {
            boxes.set(i, at(x, y, z, 0.5), true);
        }
        boxes.set(tests.len(), at(40., 0., 0., 0.5), false);
        let mut mask = Vec::new();
        visible(&frustum, &boxes, &mut mask);
        assert_eq!(mask, [true, false, false, true, false, true]);
    }

    #[test]
    fn spans_keep_short_gaps_and_ask_a_runs_indices_only_at_one() {
        let run = |geometry, instances| Run {
            geometry,
            glass: false,
            maps: [0; crate::texture::MAPS],
            variant: Default::default(),
            instances,
            casts: true,
        };
        // Run 0 all shown; run 1 a gap of 1 then of 3; run 2 none shown.
        let runs = [run(0, 0..4), run(1, 4..12), run(2, 12..14)];
        let mut mask = vec![true; 14];
        mask[5] = false;
        mask[8..11].fill(false);
        mask[12..14].fill(false);
        let asked = std::cell::RefCell::new(Vec::new());
        // Run 1's geometry has DRAW_INDICES / 2 indices: gaps of 2 kept.
        let indices = |r: &Run| {
            asked.borrow_mut().push(r.geometry);
            DRAW_INDICES / 2
        };
        let mut out = Vec::new();
        let culled = shown(&runs, Some(&mask), &mut out, indices);
        assert_eq!(out, [(0, 0..4), (1, 4..8), (1, 11..12)]);
        assert_eq!(culled, 3 + 2);
        assert_eq!(*asked.borrow(), [1], "once, at run 1's first gap");
    }
}
