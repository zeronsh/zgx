//! Point and spot lights binned into a view-space grid of clusters
//! (design-3d-aaa.md, 6.3): the view's box cut into `TILES_X` by `TILES_Y`
//! tiles and its depth into `SLICES` slices, each slice a constant ratio
//! deeper than the one before. A fragment shades with its cluster's lights
//! alone. Binned on the CPU, and only when a light, the camera or the
//! view's size changed (`Lights::prepare`): a frame where none did keeps the
//! last frame's lists on the device.
use crate::math::{Mat4, Vec3};

pub(super) const TILES_X: u32 = 16;
pub(super) const TILES_Y: u32 = 9;
pub(super) const SLICES: u32 = 24;
pub(super) const CLUSTERS: usize = (TILES_X * TILES_Y * SLICES) as usize;
/// The most lights a cluster lists (its nearest), and the most a frame
/// lists in all (the nearest the camera sees): past them, lights are left
/// out and counted (`Binned::dropped`), so the lists' buffer stays within
/// what a device binds whatever a world holds.
pub(super) const MAX_PER_CLUSTER: usize = 255;
pub(super) const MAX_LIGHTS: usize = 1 << 16;

/// A light as the shader reads it, as the uniform path's slots are: where
/// it is and how far it reaches, its colour and intensity, its cone.
pub(super) type Packed = [[f32; 4]; 3];

/// How the grid lies in the view: its projection's scales, and the depths
/// its slices span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Grid {
    /// The projection's x and y scales (`Mat4::perspective`'s `f / aspect`
    /// and `f`, or the orthographic's `2 / width` and `2 / height`).
    pub(super) scale: [f32; 2],
    pub(super) orthographic: bool,
    /// The depths in front of the camera the slices cut, the first's near
    /// side and the last's far side: where lights reach, within what the
    /// camera sees.
    pub(super) near: f32,
    pub(super) far: f32,
}
impl Grid {
    /// The slice holding depth `depth` (unclamped): logarithmic in depth
    /// under perspective, linear under an orthographic view: as the shader
    /// finds a fragment's (`cluster_span`).
    #[cfg(test)]
    fn slice_of(&self, depth: f32) -> f32 {
        let [scale, bias] = self.slicing();
        let d = if self.orthographic {
            depth
        } else {
            depth.max(1e-6).ln()
        };
        d * scale + bias
    }
    /// The depth slice `slice` starts at.
    fn depth_of(&self, slice: u32) -> f32 {
        let t = slice as f32 / SLICES as f32;
        if self.orthographic {
            self.near + (self.far - self.near) * t
        } else {
            self.near * (self.far / self.near).powf(t)
        }
    }
    /// `slice = f(depth) * scale + bias`, `f` the logarithm or not.
    pub(super) fn slicing(&self) -> [f32; 2] {
        let (near, far) = if self.orthographic {
            (self.near, self.far)
        } else {
            (self.near.max(1e-6).ln(), self.far.max(1e-6).ln())
        };
        let scale = SLICES as f32 / (far - near).max(1e-6);
        [scale, -near * scale]
    }
}

/// The lists: each cluster's lights, as indices into `shown`.
#[derive(Default)]
pub(super) struct Binned {
    /// The lights binned, as given, and those the camera sees, nearest
    /// first (`MAX_LIGHTS` at most), as the lists index them.
    pub(super) lights: Vec<Packed>,
    pub(super) shown: Vec<Packed>,
    /// Each cluster's first index in `indices` and how many, cluster by
    /// cluster: x fastest, then y (top to bottom), then depth.
    pub(super) clusters: Vec<[u32; 2]>,
    pub(super) indices: Vec<u32>,
    pub(super) grid: Option<Grid>,
    /// Each light's runs of clusters as binning found them: a slice's rows
    /// of tiles (light, slice, x from and to, y from and to).
    spans: Vec<(u32, u32, [u32; 4])>,
    /// Lights and listings left out past `MAX_LIGHTS` and
    /// `MAX_PER_CLUSTER`.
    pub(super) dropped: u32,
    /// The lights the camera may see: index, view-space centre, radius,
    /// and how far from the camera, squared (their order).
    visible: Vec<(u32, Vec3, f32, f32)>,
    /// Each cluster's room in `indices`, while filling it.
    room: Vec<u32>,
}

/// The tiles (x from, x to, y from, y to, inclusive) a light's sphere
/// `(centre, radius)` in view space covers between depths `z0` and `z1`,
/// `None` if none.
fn tiles(grid: &Grid, centre: Vec3, radius: f32, (z0, z1): (f32, f32)) -> Option<[u32; 4]> {
    let depth = -centre.z;
    // The sphere's widest within the slab, about its centre's line.
    let closest = depth.clamp(z0, z1);
    let r = (radius * radius - (depth - closest) * (depth - closest))
        .max(0.)
        .sqrt();
    let (x0, x1, y0, y1) = (centre.x - r, centre.x + r, centre.y - r, centre.y + r);
    // Normalised device coordinates' bounds: under perspective x / depth
    // is greatest and least at the slab's two faces.
    let [sx, sy] = grid.scale;
    let ([nx0, nx1], [ny0, ny1]) = if grid.orthographic {
        ([x0 * sx, x1 * sx], [y0 * sy, y1 * sy])
    } else {
        let (a, b) = (1. / z0, 1. / z1);
        (
            [(x0 * a).min(x0 * b) * sx, (x1 * a).max(x1 * b) * sx],
            [(y0 * a).min(y0 * b) * sy, (y1 * a).max(y1 * b) * sy],
        )
    };
    if nx1 < -1. || nx0 > 1. || ny1 < -1. || ny0 > 1. {
        return None;
    }
    let tile = |n: f32, tiles: u32| ((n * 0.5 + 0.5) * tiles as f32).clamp(0., tiles as f32 - 1.);
    // Rows go down the screen as y goes up.
    Some([
        tile(nx0, TILES_X) as u32,
        tile(nx1, TILES_X) as u32,
        tile(-ny1, TILES_Y) as u32,
        tile(-ny0, TILES_Y) as u32,
    ])
}

impl Binned {
    /// Bins `lights` (in the world's frame) for the camera `view` with
    /// projection scales and slicing as `grid` says, its `near` and `far`
    /// those of the camera: the grid's depths are narrowed to where the
    /// lights reach.
    pub(super) fn bin(&mut self, view: &Mat4, mut grid: Grid, lights: &[Packed]) {
        self.lights.clear();
        self.lights.extend_from_slice(lights);
        self.dropped = 0;
        // Each light the camera may see, its sphere in view space, nearest
        // first, `MAX_LIGHTS` at most.
        let visible = &mut self.visible;
        visible.clear();
        for (light, [at, ..]) in lights.iter().enumerate() {
            let centre = view.project(Vec3::from([at[0], at[1], at[2]]));
            let (radius, depth) = (at[3], -centre.z);
            let reach = (
                (depth - radius).max(grid.near),
                (depth + radius).min(grid.far),
            );
            // (Not a number reaches nowhere: compared, it isn't less.)
            let reaches = reach.0 < reach.1;
            if !reaches || tiles(&grid, centre, radius, reach).is_none() {
                continue;
            }
            visible.push((light as u32, centre, radius, centre.dot(centre)));
        }
        // Nearest first, where a cap could leave some out: with no more
        // than a cluster holds, none can be, and their order is no matter.
        if visible.len() > MAX_PER_CLUSTER {
            visible.sort_unstable_by(|a, b| a.3.total_cmp(&b.3));
        }
        if visible.len() > MAX_LIGHTS {
            self.dropped += (visible.len() - MAX_LIGHTS) as u32;
            visible.truncate(MAX_LIGHTS);
        }
        self.shown.clear();
        self.shown
            .extend(visible.iter().map(|&(light, ..)| lights[light as usize]));
        // The depths they reach: the grid's slices span those alone.
        let (mut near, mut far) = (f32::MAX, f32::MIN);
        for &(_, centre, radius, _) in visible.iter() {
            near = near.min(-centre.z - radius);
            far = far.max(-centre.z + radius);
        }
        grid.near = near.max(grid.near);
        grid.far = far.min(grid.far);
        let counts = &mut self.clusters;
        counts.clear();
        counts.resize(CLUSTERS, [0; 2]);
        self.spans.clear();
        self.indices.clear();
        if grid.near >= grid.far {
            // No light reaches what the camera sees.
            grid.far = grid.near * 2.;
            self.grid = Some(grid);
            return;
        }
        self.grid = Some(grid);
        let at = |x: u32, y: u32, z: u32| ((z * TILES_Y + y) * TILES_X + x) as usize;
        // Where each slice starts, once: a power apiece.
        let mut depths = [0.; SLICES as usize + 1];
        for (z, depth) in depths.iter_mut().enumerate() {
            *depth = grid.depth_of(z as u32);
        }
        for (light, &(_, centre, radius, _)) in visible.iter().enumerate() {
            let light = light as u32;
            let depth = -centre.z;
            // The slices holding its nearest and farthest: found among the
            // slices' depths (no logarithm a light).
            let slice = |d: f32| (depths[1..SLICES as usize].partition_point(|&s| s <= d)) as u32;
            let (first, last) = (slice(depth - radius), slice(depth + radius));
            for z in first..=last {
                let slab = (depths[z as usize], depths[z as usize + 1]);
                let Some(rect) = tiles(&grid, centre, radius, slab) else {
                    continue;
                };
                let [x0, x1, y0, y1] = rect;
                for y in y0..=y1 {
                    for cluster in &mut counts[at(x0, y, z)..=at(x1, y, z)] {
                        cluster[1] += 1;
                    }
                }
                self.spans.push((light, z, rect));
            }
        }
        // Each cluster's first index and its room (its count, at most
        // `MAX_PER_CLUSTER`), then its lights nearest first: the farthest
        // past its room left out. (Checked a listing at a time only where
        // some cluster is over: almost never.)
        let mut total = 0;
        let mut over = false;
        for cluster in counts.iter_mut() {
            let fits = cluster[1].min(MAX_PER_CLUSTER as u32);
            over |= fits < cluster[1];
            self.dropped += cluster[1] - fits;
            cluster[0] = total;
            cluster[1] = fits;
            total += fits;
        }
        self.indices.resize(total as usize, 0);
        let indices = &mut self.indices[..];
        if over {
            let room = &mut self.room;
            room.clear();
            room.extend(counts.iter().map(|cluster| cluster[1]));
            counts.iter_mut().for_each(|cluster| cluster[1] = 0);
            for &(light, z, [x0, x1, y0, y1]) in &self.spans {
                for y in y0..=y1 {
                    let row = at(x0, y, z)..=at(x1, y, z);
                    for (cluster, &room) in counts[row.clone()].iter_mut().zip(&room[row]) {
                        if cluster[1] < room {
                            indices[(cluster[0] + cluster[1]) as usize] = light;
                            cluster[1] += 1;
                        }
                    }
                }
            }
            return;
        }
        // Where each cluster's next index goes: its count, counted back up.
        counts.iter_mut().for_each(|cluster| cluster[1] = 0);
        for &(light, z, [x0, x1, y0, y1]) in &self.spans {
            for y in y0..=y1 {
                for cluster in &mut counts[at(x0, y, z)..=at(x1, y, z)] {
                    indices[(cluster[0] + cluster[1]) as usize] = light;
                    cluster[1] += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, vec3};

    fn grid(camera: &Camera, aspect: f32) -> Grid {
        let projection = camera.projection(aspect);
        Grid {
            scale: [projection.columns[0][0], projection.columns[1][1]],
            orthographic: camera.orthographic,
            near: camera.near,
            far: camera.far,
        }
    }

    /// A light at `p` reaching `range`.
    fn light(p: Vec3, range: f32) -> Packed {
        [[p.x, p.y, p.z, range], [1.; 4], [0., 0., 0., 1.]]
    }

    /// Lights scattered about, some behind the camera and some past the
    /// screen's sides.
    fn scattered(n: usize) -> Vec<Packed> {
        let mut state = 12345u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 10_000) as f32 / 10_000.
        };
        (0..n)
            .map(|_| {
                let p = vec3(-30. + 60. * next(), -5. + 10. * next(), -50. + 60. * next());
                light(p, 0.5 + 6. * next())
            })
            .collect()
    }

    /// Whether the sphere `(centre, radius)` (view space) meets the part of
    /// the view cluster `(x, y, z)` holds, sampled: points through the
    /// cluster's volume, by its tiles' pixels and its slice's depths.
    fn touches(grid: &Grid, (centre, radius): (Vec3, f32), (x, y, z): (u32, u32, u32)) -> bool {
        let steps = 6;
        let (z0, z1) = (grid.depth_of(z), grid.depth_of(z + 1));
        for i in 0..=steps {
            for j in 0..=steps {
                for k in 0..=steps {
                    let t =
                        |v: u32, i: u32, n: u32| (v as f32 + i as f32 / steps as f32) / n as f32;
                    let nx = t(x, i, TILES_X) * 2. - 1.;
                    let ny = 1. - t(y, j, TILES_Y) * 2.;
                    let depth = z0 + (z1 - z0) * k as f32 / steps as f32;
                    let (px, py) = if grid.orthographic {
                        (nx / grid.scale[0], ny / grid.scale[1])
                    } else {
                        (nx * depth / grid.scale[0], ny * depth / grid.scale[1])
                    };
                    let p = vec3(px, py, -depth);
                    if (p - centre).length() < radius * 0.999 {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[test]
    fn every_cluster_a_light_reaches_lists_it() {
        for orthographic in [false, true] {
            let camera = Camera {
                orthographic,
                fov: if orthographic { 30. } else { 0.9 },
                far: 80.,
                ..Camera::looking_at([0., 2., 8.], [0., 0., -10.])
            };
            let aspect = 16. / 9.;
            let view = camera.view();
            let lights = scattered(64);
            let mut binned = Binned::default();
            binned.bin(&view, grid(&camera, aspect), &lights);
            let grid = binned.grid.unwrap();
            let mut listed = 0;
            for z in 0..SLICES {
                for y in 0..TILES_Y {
                    for x in 0..TILES_X {
                        let cluster = ((z * TILES_Y + y) * TILES_X + x) as usize;
                        let [first, count] = binned.clusters[cluster];
                        let list: Vec<u32> = binned.indices
                            [first as usize..(first + count) as usize]
                            .iter()
                            .map(|&at| binned.visible[at as usize].0)
                            .collect();
                        listed += list.len();
                        for (i, l) in lights.iter().enumerate() {
                            let centre = view.project(vec3(l[0][0], l[0][1], l[0][2]));
                            if touches(&grid, (centre, l[0][3]), (x, y, z)) {
                                assert!(
                                    list.contains(&(i as u32)),
                                    "light {i} reaches cluster {x} {y} {z} (orthographic: {orthographic})"
                                );
                            }
                        }
                    }
                }
            }
            // Not every light in every cluster: binning narrows them down.
            assert!(
                listed > 0 && listed < lights.len() * CLUSTERS / 20,
                "{listed}"
            );
        }
    }

    #[test]
    fn a_cluster_lists_its_nearest_lights_up_to_its_cap_and_counts_the_rest() {
        let camera = Camera::looking_at([0., 0., 0.], [0., 0., -1.]);
        // More lights than a cluster holds, each reaching every cluster.
        let count = MAX_PER_CLUSTER + 40;
        let lights: Vec<Packed> = (0..count)
            .map(|i| light(vec3(0., 0., -10. - i as f32 * 0.01), 1000.))
            .collect();
        let mut binned = Binned::default();
        binned.bin(&camera.view(), grid(&camera, 1.), &lights);
        for &[first, n] in &binned.clusters {
            assert!(n as usize <= MAX_PER_CLUSTER);
            // The nearest kept: the farthest (the last given) dropped.
            let list = &binned.indices[first as usize..(first + n) as usize];
            assert!(
                list.iter().all(|&l| (l as usize) < MAX_PER_CLUSTER),
                "{list:?}"
            );
        }
        // The 40 farthest, left out of every cluster.
        assert_eq!(binned.dropped as usize, 40 * CLUSTERS);
    }

    #[test]
    fn the_slices_span_where_the_lights_reach() {
        let camera = Camera::looking_at([0., 0., 0.], [0., 0., -1.]);
        let mut binned = Binned::default();
        let lights = [light(vec3(0., 0., -10.), 2.), light(vec3(1., 0., -30.), 5.)];
        binned.bin(&camera.view(), grid(&camera, 1.), &lights);
        let grid = binned.grid.unwrap();
        assert!((grid.near - 8.).abs() < 1e-4 && (grid.far - 35.).abs() < 1e-4);
        assert!((grid.slice_of(8.)).abs() < 1e-4);
        assert!((grid.slice_of(35.) - SLICES as f32).abs() < 1e-3);
        // Behind the camera, nothing.
        binned.bin(
            &camera.view(),
            self::grid(&camera, 1.),
            &[light(vec3(0., 0., 10.), 2.)],
        );
        assert!(binned.indices.is_empty());
    }

    /// How long binning the comparison's light scene takes (256 lights
    /// reaching 8 over Bistro's quarter, from its camera path's start): the
    /// CPU side of the CPU-against-compute measurement. Run with
    /// `cargo test --release --features lighting --lib time_binning -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_binning() {
        let camera = Camera {
            far: 400.,
            ..Camera::looking_at([95., 45., 4.], [25., 0., 4.])
        };
        let lights: Vec<Packed> = (0..256u32)
            .map(|i| {
                let mut state = i.wrapping_mul(2_654_435_761).wrapping_add(12345);
                let mut next = || {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    (state % 10_000) as f32 / 10_000.
                };
                let p = vec3(-40. + 130. * next(), 1. + 5. * next(), -60. + 130. * next());
                light(p, 8.)
            })
            .collect();
        let mut binned = Binned::default();
        let (grid, view) = (grid(&camera, 16. / 9.), camera.view());
        let n: u32 = std::env::var("BIN_N")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(2000);
        let start = std::time::Instant::now();
        for _ in 0..n {
            binned.bin(&view, grid, &lights);
        }
        let us = start.elapsed().as_secs_f64() * 1e6 / n as f64;
        eprintln!(
            "bin: {us:.1} us, {} indices, {} spans",
            binned.indices.len(),
            binned.spans.len()
        );
    }
}
