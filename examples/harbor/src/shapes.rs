//! Geometry made in code: oriented beams, corrugated containers, a lofted
//! hull and a far ridge, each from `Geometry::new` or the stock shapes.
use zgx_3d::{Geometry, Quat, Transform, Vec3, Vertex, vec3};

/// The rotation turning `from` onto `to` (both non-zero).
pub fn quat_between(from: Vec3, to: Vec3) -> Quat {
    let (a, b) = (from.normalized(), to.normalized());
    let d = a.dot(b).clamp(-1., 1.);
    if d > 0.999_999 {
        return Quat::IDENTITY;
    }
    if d < -0.999_999 {
        // Half a turn about any axis square to `a`.
        let axis = if a.x.abs() < 0.9 {
            a.cross(vec3(1., 0., 0.))
        } else {
            a.cross(vec3(0., 1., 0.))
        };
        return Quat::from_axis_angle(axis.normalized(), std::f32::consts::PI);
    }
    Quat::from_axis_angle(a.cross(b).normalized(), d.acos())
}

/// Where a unit cube goes to be a beam from `a` to `b`, `width` by `depth`
/// across (its depth along the horizontal square to it where it can).
pub fn beam(a: Vec3, b: Vec3, width: f32, depth: f32) -> Transform {
    let along = b - a;
    let length = along.length().max(1e-4);
    Transform {
        position: (a + b) * 0.5,
        rotation: quat_between(Vec3::Y, along),
        scale: vec3(width, length, depth),
    }
}

/// A box `size` with its bottom's middle at `position`.
pub fn block(position: Vec3, size: Vec3) -> Transform {
    Transform {
        position: position + vec3(0., size.y * 0.5, 0.),
        rotation: Quat::IDENTITY,
        scale: size,
    }
}

/// Triangles collected with their own flat normals, or shared ones
/// smoothed across the faces meeting at each vertex.
#[derive(Default)]
pub struct Builder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl Builder {
    /// A flat quad `a b c d`, counter-clockwise seen from its front, its
    /// texture coordinates `uv` in the same order.
    pub fn quad(&mut self, corners: [Vec3; 4], uv: [[f32; 2]; 4]) {
        let [a, b, c, d] = corners;
        let normal = (b - a).cross(c - a).normalized();
        let base = self.vertices.len() as u32;
        for (p, uv) in corners.iter().zip(uv) {
            self.vertices.push(Vertex {
                position: p.to_array(),
                normal: normal.to_array(),
                uv,
            });
        }
        let _ = d;
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    /// A grid of `rows` by `columns` points (row-major), each quad two
    /// triangles wound `(r, c) (r, c+1) (r+1, c+1)`; normals are smoothed
    /// over the faces at each point.
    pub fn grid(&mut self, rows: usize, columns: usize, points: &[(Vec3, [f32; 2])]) {
        assert_eq!(points.len(), rows * columns);
        let base = self.vertices.len() as u32;
        for &(p, uv) in points {
            self.vertices.push(Vertex {
                position: p.to_array(),
                normal: [0.; 3],
                uv,
            });
        }
        let first = self.indices.len();
        for r in 0..rows - 1 {
            for c in 0..columns - 1 {
                let i = |r: usize, c: usize| base + (r * columns + c) as u32;
                self.indices.extend([
                    i(r, c),
                    i(r, c + 1),
                    i(r + 1, c + 1),
                    i(r, c),
                    i(r + 1, c + 1),
                    i(r + 1, c),
                ]);
            }
        }
        // Each face's normal, weighted by its area, summed at its corners.
        let mut sums = vec![Vec3::ZERO; points.len()];
        for t in self.indices[first..].chunks(3) {
            let p = |k: usize| Vec3::from(self.vertices[t[k] as usize].position);
            let n = (p(1) - p(0)).cross(p(2) - p(0));
            for &k in t {
                sums[(k - base) as usize] = sums[(k - base) as usize] + n;
            }
        }
        for (k, n) in sums.into_iter().enumerate() {
            let n = if n.length() > 1e-12 {
                n.normalized()
            } else {
                Vec3::Y
            };
            self.vertices[base as usize + k].normal = n.to_array();
        }
    }
    /// The geometry.
    pub fn build(self) -> Geometry {
        Geometry::new(self.vertices, self.indices).expect("indices made in range")
    }
    /// Its vertices so far.
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }
}

/// A shipping container's length (m): 20 and 40 foot.
pub const CONTAINER_20: f32 = 6.058;
/// A 40 foot container's length.
pub const CONTAINER_40: f32 = 12.192;
/// A container's width.
pub const CONTAINER_WIDTH: f32 = 2.438;
/// A container's height (high cube: 2.896).
pub const CONTAINER_HEIGHT: f32 = 2.591;

/// A container `length` long along X, its bottom's middle at the origin:
/// corrugated long sides (trapezoid ribs, flat-shaded so their edges
/// catch the sun), flat doors and ends, a flat roof. Texture coordinates
/// run along each side (U) and down it (V).
pub fn container(length: f32) -> Geometry {
    let (w, h) = (CONTAINER_WIDTH * 0.5, CONTAINER_HEIGHT);
    let depth = 0.036;
    let period = 0.278;
    let ribs = (length / period).round() as usize;
    let step = length / ribs as f32;
    // A rib's profile along X: out, slope in, in, slope out.
    let profile: Vec<(f32, f32)> = (0..=ribs * 4)
        .map(|k| {
            let x = -length * 0.5 + k as f32 * step / 4.;
            let out = matches!(k % 4, 0 | 1);
            (x, if out { 0. } else { -depth })
        })
        .collect();
    let mut b = Builder::default();
    for side in [1f32, -1.] {
        for pair in profile.windows(2) {
            let ((x0, d0), (x1, d1)) = (pair[0], pair[1]);
            let (z0, z1) = (side * (w + d0), side * (w + d1));
            let (u0, u1) = ((x0 / length) + 0.5, (x1 / length) + 0.5);
            let (u0, u1) = if side > 0. {
                (u0, u1)
            } else {
                (1. - u0, 1. - u1)
            };
            let corners = [
                vec3(x0, 0., z0),
                vec3(x1, 0., z1),
                vec3(x1, h, z1),
                vec3(x0, h, z0),
            ];
            let uv = [[u0, 1.], [u1, 1.], [u1, 0.], [u0, 0.]];
            if side > 0. {
                b.quad(corners, uv);
            } else {
                let [a, bb, c, d] = corners;
                let [ua, ub, uc, ud] = uv;
                b.quad([bb, a, d, c], [ub, ua, ud, uc]);
            }
        }
    }
    let l = length * 0.5;
    // Doors (+X) and the far end (-X).
    b.quad(
        [
            vec3(l, 0., w),
            vec3(l, 0., -w),
            vec3(l, h, -w),
            vec3(l, h, w),
        ],
        [[0., 1.], [0.2, 1.], [0.2, 0.], [0., 0.]],
    );
    b.quad(
        [
            vec3(-l, 0., -w),
            vec3(-l, 0., w),
            vec3(-l, h, w),
            vec3(-l, h, -w),
        ],
        [[0.2, 1.], [0.4, 1.], [0.4, 0.], [0.2, 0.]],
    );
    // Roof.
    b.quad(
        [
            vec3(-l, h, w),
            vec3(l, h, w),
            vec3(l, h, -w),
            vec3(-l, h, -w),
        ],
        [[0., 0.], [1., 0.], [1., 0.1], [0., 0.1]],
    );
    b.build()
}

/// A ship's hull `length` along X and `beam` across, its deck at `deck`
/// and keel at `keel` (m), about X = 0 (+X the bow). Lofted from
/// superellipse sections, stations packed toward the ends where the shape
/// changes: boxy amidships; toward the stern the run rises to a transom
/// whose foot is just under the water line (`boot` is the boot top's
/// lower edge), the sections turning to a rounded V over the skeg; toward
/// the bow the sheer rises, the forefoot lifts and the sections sharpen.
/// The transom is a flat panel closed by strips, the deck a strip
/// between the deck edges. Vertex colours paint the bottom red below the
/// boot top, the boot top black, the topsides `topsides`.
pub fn hull(
    length: f32,
    beam: f32,
    deck: f32,
    keel: f32,
    boot: f32,
    topsides: [f32; 4],
) -> Geometry {
    let stations = 96;
    let half = 16; // section points from the deck edge to the keel
    // The transom's foot: a metre under the boot top's lower edge.
    let transom_foot = boot - 1.;
    let section = |s: f32| -> Vec<Vec3> {
        let x = -length * 0.5 + s * length;
        let bow = ((s - 0.72) / 0.28).clamp(0., 1.);
        // The run: over the last 18% toward the stern the bottom rises.
        let run = ((0.18 - s) / 0.18).clamp(0., 1.);
        let run = run * run * (3. - 2. * run);
        let b = beam * 0.5 * (1. - bow * bow).max(0.).sqrt().max(0.02) * (1. - 0.1 * run);
        let top = deck + 2.5 * bow * bow + 0.6 * run;
        let bottom = keel + (deck - keel) * 0.55 * bow.powf(3.) + (transom_foot - keel) * run;
        // Boxy amidships, a rounded V at the ends.
        let n = 7. - 4.5 * bow - 4.2 * run;
        (0..=half)
            .map(|k| {
                let phi = k as f32 / half as f32 * std::f32::consts::FRAC_PI_2;
                let (c, sn) = (phi.cos().max(0.), phi.sin().max(0.));
                let z = b * c.powf(2. / n);
                let y = top - (top - bottom) * sn.powf(2. / n);
                vec3(x, y, z)
            })
            .collect()
    };
    let columns = half * 2 + 1;
    let mut points = Vec::with_capacity((stations + 1) * columns);
    let mut tops = Vec::new();
    let mut transom = Vec::new();
    for i in 0..=stations {
        // Packed toward both ends (cosine spacing).
        let s = 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / stations as f32).cos();
        let one = section(s);
        // Starboard (+Z) from the deck edge down to the keel, then port up.
        let mut row: Vec<Vec3> = one.clone();
        row.extend(one.iter().rev().skip(1).map(|p| vec3(p.x, p.y, -p.z)));
        for (c, p) in row.iter().enumerate() {
            points.push((*p, [s, c as f32 / (columns - 1) as f32]));
        }
        tops.push((one[0], vec3(one[0].x, one[0].y, -one[0].z)));
        if i == 0 {
            transom = one;
        }
    }
    let mut b = Builder::default();
    b.grid(stations + 1, columns, &points);
    // Deck: a strip between the two deck edges, station by station.
    for pair in tops.windows(2) {
        let ((s0, p0), (s1, p1)) = (pair[0], pair[1]);
        b.quad([s0, s1, p1, p0], [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]);
    }
    // Transom: a flat panel facing aft, closed by strips from starboard to
    // port, row by row down the stern's section.
    for pair in transom.windows(2) {
        let (p0, p1) = (pair[0], pair[1]);
        let (q0, q1) = (vec3(p0.x, p0.y, -p0.z), vec3(p1.x, p1.y, -p1.z));
        b.quad([p0, q0, q1, p1], [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]);
    }
    let colors = b
        .vertices()
        .iter()
        .map(|v| {
            if v.position[1] < boot {
                [0.32, 0.06, 0.04, 1.]
            } else if v.position[1] < boot + 1.2 {
                [0.03, 0.03, 0.035, 1.]
            } else {
                topsides
            }
        })
        .collect();
    b.build().with_colors(colors).expect("a colour a vertex")
}

/// A far ridge of hills along X at `z`, from the water at `base` up to
/// about `height`, `width` wide: the shore across the water.
pub fn ridge(z: f32, width: f32, base: f32, height: f32, seed: u32) -> Geometry {
    let (columns, rows) = (160, 6);
    let mut points = Vec::with_capacity(columns * rows);
    for r in 0..rows {
        // Rows go from the near foot (t = 1) up to the far crest (t = 0),
        // so the grid's triangles wind toward the harbor.
        let t = 1. - r as f32 / (rows - 1) as f32;
        for c in 0..columns {
            let u = c as f32 / (columns - 1) as f32;
            let x = -width * 0.5 + u * width;
            let crest = height * (0.35 + 0.65 * crate::rng::fbm2(u * 9., 0.5, 4, seed));
            let y = base + crest * (1. - t).powf(1.4);
            let depth = z - 140. * (1. - t);
            points.push((vec3(x, y, depth), [u, t]));
        }
    }
    let mut b = Builder::default();
    b.grid(rows, columns, &points);
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each triangle's winding agrees with its vertices' normals.
    fn wound_toward_normals(g: &Geometry) {
        let v = g.vertices();
        for t in g.indices().chunks(3) {
            let p = |k: usize| Vec3::from(v[t[k] as usize].position);
            let n = (p(1) - p(0)).cross(p(2) - p(0));
            if n.length() < 1e-9 {
                continue;
            }
            let normal = Vec3::from(v[t[0] as usize].normal);
            assert!(n.dot(normal) > 0., "a triangle wound against its normal");
        }
    }

    #[test]
    fn a_beam_runs_from_end_to_end() {
        let (a, b) = (vec3(1., 2., 3.), vec3(4., 6., 3.));
        let t = beam(a, b, 0.5, 0.5);
        assert!((t.scale.y - 5.).abs() < 1e-5);
        let tip = t.rotation.rotate(vec3(0., 0.5 * t.scale.y, 0.)) + t.position;
        assert!((tip - b).length() < 1e-4, "{tip:?}");
    }

    #[test]
    fn containers_face_outward() {
        for length in [CONTAINER_20, CONTAINER_40] {
            let g = container(length);
            wound_toward_normals(&g);
            let (low, high) = g.bounds();
            assert!((high.x - low.x - length).abs() < 1e-3);
            assert!((high.y - CONTAINER_HEIGHT).abs() < 1e-4 && low.y.abs() < 1e-6);
        }
    }

    #[test]
    fn the_hull_faces_outward() {
        let g = hull(220., 32., 9., -14., -3., [0.02, 0.05, 0.1, 1.]);
        // Smoothed normals face the way the triangles wind: outward if the
        // starboard side's points, amidships, face +Z.
        let mid = g
            .vertices()
            .iter()
            .filter(|v| {
                v.position[0].abs() < 5.
                    && v.position[2] > 15.
                    && v.position[1] > 0.
                    && v.normal[1] < 0.99
            })
            .collect::<Vec<_>>();
        assert!(!mid.is_empty());
        let bad: Vec<_> = mid
            .iter()
            .filter(|v| v.normal[2] <= 0.5)
            .map(|v| (v.position, v.normal))
            .collect();
        assert!(bad.is_empty(), "the side faces out: {bad:?}");
        let deck = g.vertices().iter().filter(|v| v.normal[1] > 0.99).count();
        assert!(deck > 0, "the deck faces up");
        // The transom is a flat panel facing aft, its foot under the water
        // and none of it below the boot top's band but that.
        let aft: Vec<_> = g
            .vertices()
            .iter()
            .filter(|v| v.normal[0] < -0.99)
            .collect();
        assert!(!aft.is_empty(), "a transom facing aft");
        let foot = aft.iter().map(|v| v.position[1]).fold(f32::MAX, f32::min);
        assert!((foot - (-4.)).abs() < 0.1, "its foot: {foot}");
        wound_toward_normals(&g);
    }

    #[test]
    fn the_ridge_faces_the_harbor() {
        let g = ridge(-1400., 4000., -3., 120., 5);
        wound_toward_normals(&g);
        // The harbor is toward +Z: its slopes face it.
        let facing = g.vertices().iter().filter(|v| v.normal[2] > 0.).count();
        assert!(facing * 10 > g.vertices().len() * 9);
    }
}
