//! Probes updated without ray queries (`gi` feature): where the device has
//! compute but no ray queries (Direct3D 12 compiling with FXC, macOS before
//! 15, Vulkan without `VK_KHR_ray_query`). The scene rays would see
//! (`raytrace::traced`) is kept as the tracer keeps it: each geometry's
//! vertices and indices once, in its own space (uploaded as it comes,
//! dropped as it goes), and each mesh as an instance (its place, normals'
//! turn, colour and glow) in one buffer, which is all a move rewrites.
//! Each of a frame's probes draws it into six faces of a cube about
//! itself (a draw a geometry a face), lit as a ray's hit is, and a compute
//! pass reads
//! the faces along the probe's rays into the rays buffer the blend reads
//! (`probes_raster.wgsl`). From there it's the tracer's path.
//!
//! What differs from rays: point and spot lights don't cast shadows on
//! what the faces see (the meshes drawn directly don't either), and the
//! sun's shadow is the frame's map or cascades as last drawn: past the
//! last cascade nothing is in shadow, so a face seeing an interior farther
//! off than the cascades reach lights it through its roof, as the meshes
//! drawn directly are.
//! A face is 16 by 16 texels: a ray reads the texel its direction falls in.
//!
//! A face draws only what can show in it: each geometry over `SPLIT`
//! triangles is cut into clusters of about `CLUSTER` (its indices
//! reordered once, as uploaded), and each frame every (mesh, cluster)
//! whose sphere reaches into a face's frustum, within `cut` of its probe,
//! is drawn there; beyond, a face sees the sky as a ray that missed
//! does. A frame updates as many of its probes as fit `BUDGET` triangles
//! drawn (one at least): a big scene converges over more frames, not
//! slower ones. The spheres bound the geometry as uploaded, which is what
//! the faces (and rays) see: neither runs a custom material's `displace`.
//! Should they, a cluster's sphere grows by its `displaces_by`.
use super::raytrace::{Held, held, traced};
use crate::world::Drawn;
use std::collections::HashMap;
use zgx_gpu::wgpu;

/// A face's side, in texels (`FACE` in `probes_raster.wgsl`).
const FACE: u32 = 16;
/// The most probes a frame draws faces for: the faces' atlas grows to as
/// many rows of six faces as a frame draws, this many at most (a 96 by
/// 4,096 atlas, 8 MB with its depth).
pub(super) const MOST: u32 = 256;
/// A face texel nothing was drawn into (`EMPTY`).
const EMPTY: f64 = 1e30;
/// Geometries of more triangles than this are cut into clusters of about
/// `CLUSTER` each.
const SPLIT: usize = 4096;
const CLUSTER: usize = 1024;
/// The most triangles a frame's faces draw, as its probes are chosen (one
/// probe at least): what bounds a frame's cost on the face path.
pub(super) const BUDGET: u64 = 8_000_000;
/// How far a face draws: this many times the probes' farthest distance
/// counted (`Layout::far`), and no nearer than `CUT_LEAST` metres.
const CUT: f32 = 2.;
const CUT_LEAST: f32 = 64.;
/// How far a face draws for probes whose distances count to `far`.
pub(super) fn cut(far: f32) -> f32 {
    (far * CUT).max(CUT_LEAST)
}

/// A mesh as the faces draw it: its place (a 3 by 4 matrix's rows), its
/// normals' turn (the inverse transpose's rows), its colour (less what's
/// metal) and glow (`capture_vertex`'s instance inputs).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Instance {
    place: [[f32; 4]; 3],
    turn: [[f32; 4]; 3],
    albedo: [f32; 4],
    glow: [f32; 4],
}
/// The bytes between faces' entries in the faces' uniform: a dynamic
/// offset's alignment.
const FACE_STRIDE: u64 = 256;

/// A run of a geometry's triangles a face culls on its own: where they
/// start and how many, in its reordered indices, and a sphere about them
/// in its own space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cluster {
    first: u32,
    triangles: u32,
    center: [f32; 3],
    radius: f32,
}

/// `indices` (triangles of `vertices`) as clusters: one where there are
/// `SPLIT` triangles or fewer; else halved at the median of their centres
/// along the widest axis until each holds `CLUSTER` or fewer. The indices
/// reordered so each cluster's triangles run together, and the clusters.
fn clustered(vertices: &[crate::Vertex], indices: &[u32]) -> (Vec<u32>, Vec<Cluster>) {
    let triangles = indices.len() / 3;
    let corner = |t: u32, k: usize| vertices[indices[t as usize * 3 + k] as usize].position;
    let centre = |t: u32| -> [f32; 3] {
        let (a, b, c) = (corner(t, 0), corner(t, 1), corner(t, 2));
        std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.)
    };
    let mut order: Vec<u32> = (0..triangles as u32).collect();
    let mut runs = Vec::new();
    let mut stack = Vec::new();
    stack.push(0..triangles);
    while let Some(run) = stack.pop() {
        let limit = if run.len() == triangles {
            SPLIT
        } else {
            CLUSTER
        };
        if run.len() <= limit {
            runs.push(run);
            continue;
        }
        let part = &mut order[run.clone()];
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &t in part.iter() {
            let c = centre(t);
            for i in 0..3 {
                lo[i] = lo[i].min(c[i]);
                hi[i] = hi[i].max(c[i]);
            }
        }
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap();
        let mid = part.len() / 2;
        part.select_nth_unstable_by(mid, |&a, &b| {
            centre(a)[axis].total_cmp(&centre(b)[axis]).then(a.cmp(&b))
        });
        // The second half first off the stack after the first: in order.
        stack.push(run.start + mid..run.end);
        stack.push(run.start..run.start + mid);
    }
    let mut reordered = Vec::with_capacity(indices.len());
    let clusters = runs
        .into_iter()
        .map(|run| {
            let first = (reordered.len() / 3) as u32;
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for &t in &order[run.clone()] {
                for k in 0..3 {
                    reordered.push(indices[t as usize * 3 + k]);
                    let p = corner(t, k);
                    for i in 0..3 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
            }
            if run.is_empty() {
                (lo, hi) = ([0.; 3], [0.; 3]);
            }
            let half: [f32; 3] = std::array::from_fn(|i| (hi[i] - lo[i]) * 0.5);
            Cluster {
                first,
                triangles: run.len() as u32,
                center: std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5),
                radius: (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt(),
            }
        })
        .collect();
    (reordered, clusters)
}

/// A geometry's vertices (`Vertex`) and its indices in clusters, uploaded
/// once.
struct Shape {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    clusters: Vec<Cluster>,
    /// The last rebuild that used it.
    used: u64,
}

/// What faces may draw: each mesh's clusters, as spheres in the world, in
/// the order draws take them (by geometry, cluster, then mesh), so what a
/// face keeps falls into runs a draw each.
#[derive(Default)]
struct Items {
    /// Centre and radius.
    spheres: Vec<[f32; 4]>,
    /// The mesh (its instance) and its geometry's cluster.
    instance: Vec<u32>,
    geometry: Vec<u64>,
    cluster: Vec<u32>,
    triangles: Vec<u32>,
}

/// One draw of a face: a geometry's cluster, for the meshes `kept[range]`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Draw {
    face: u32,
    geometry: u64,
    cluster: u32,
    kept: std::ops::Range<u32>,
}

/// A frame's faces as culled (`plan`): the meshes each draw takes, the
/// draws, how many probes they're for and the triangles they draw.
#[derive(Default, Debug)]
pub(super) struct Plan {
    kept: Vec<u32>,
    draws: Vec<Draw>,
    pub(super) probes: u32,
    pub(super) triangles: u64,
}

/// `place`'s relation to a sphere (`rel`: its centre less `place`; radius
/// `r`): whether any of it may show in cube face `face` (as `on_face` in
/// `probes_raster.wgsl` turns it), conservatively.
fn in_face(face: u32, rel: [f32; 3], r: f32) -> bool {
    let (u, v, w) = match face {
        0 => (-rel[2], -rel[1], rel[0]),
        1 => (rel[2], -rel[1], -rel[0]),
        2 => (rel[0], rel[2], rel[1]),
        3 => (rel[0], -rel[2], -rel[1]),
        4 => (rel[0], -rel[1], rel[2]),
        _ => (-rel[0], -rel[1], -rel[2]),
    };
    // The face's four side planes run through its edges at 45 degrees; a
    // sphere is past one where its centre is more than r beyond it.
    let r = r * std::f32::consts::SQRT_2;
    w + r > 0. && u.abs() - w <= r && v.abs() - w <= r
}

/// The faces of the probes at `places` (this frame's, in turn) culled
/// against `items` within `cut` of each, as many probes as fit `budget`
/// triangles (one at least).
fn plan(items: &Items, places: &[[f32; 3]], cut: f32, budget: u64, out: &mut Plan) {
    out.kept.clear();
    out.draws.clear();
    out.probes = 0;
    out.triangles = 0;
    let mut near = Vec::new();
    for (probe, place) in places.iter().enumerate() {
        let (kept, draws, triangles) = (out.kept.len(), out.draws.len(), out.triangles);
        near.clear();
        for (i, s) in items.spheres.iter().enumerate() {
            let rel = [s[0] - place[0], s[1] - place[1], s[2] - place[2]];
            let reach = cut + s[3];
            if rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2] <= reach * reach {
                near.push((i, rel, s[3]));
            }
        }
        for side in 0..6 {
            let face = probe as u32 * 6 + side;
            for &(i, rel, r) in &near {
                if !in_face(side, rel, r) {
                    continue;
                }
                let (geometry, cluster) = (items.geometry[i], items.cluster[i]);
                let at = out.kept.len() as u32;
                match out.draws.last_mut() {
                    Some(d)
                        if d.face == face
                            && d.geometry == geometry
                            && d.cluster == cluster
                            && d.kept.end == at =>
                    {
                        d.kept.end += 1;
                    }
                    _ => out.draws.push(Draw {
                        face,
                        geometry,
                        cluster,
                        kept: at..at + 1,
                    }),
                }
                out.kept.push(items.instance[i]);
                out.triangles += u64::from(items.triangles[i]);
            }
        }
        if out.triangles > budget && probe > 0 {
            out.kept.truncate(kept);
            out.draws.truncate(draws);
            out.triangles = triangles;
            break;
        }
        out.probes += 1;
    }
}

pub(super) struct Raster {
    /// The world's meshes it holds (`Inner::meshes`); `None` before any.
    pub(super) meshes: Option<u64>,
    held: Vec<Held>,
    seen: Vec<Held>,
    /// Counts the times what the faces see changed: the probes converge
    /// again.
    pub(super) revision: u64,
    rebuilds: u64,
    shapes: HashMap<u64, Shape>,
    /// The meshes as instances, grouped by geometry: the buffer and how
    /// many it holds; and each geometry's run of them.
    instances: Option<(wgpu::Buffer, u32)>,
    /// Each mesh's clusters as faces cull them, and this frame's faces as
    /// culled; the meshes each draw takes, uploaded (its size in entries).
    items: Items,
    plan: Plan,
    kept: Option<(wgpu::Buffer, u32)>,
    /// Each face's index at its own offset (`FACE_STRIDE`); the group's
    /// layout, and its binding with the instances and the kept list, for
    /// the buffers they were made with.
    face_buffer: Option<wgpu::Buffer>,
    face_layout: Option<wgpu::BindGroupLayout>,
    face: Option<wgpu::BindGroup>,
    /// Buffers made and instance bytes written, counted (for tests); and
    /// the bytes of the geometries' reordered indices and cluster tables.
    pub(super) buffers_made: u64,
    pub(super) instance_bytes: u64,
    pub(super) cluster_bytes: u64,
    /// Frames whose faces were culled (for tests: none at rest).
    pub(super) plans: u64,
    /// The faces' atlas (a row of six faces a probe drawn a frame), its
    /// depth and rows, made with the first frame; and how many times it
    /// was made (what binds it is made again with it).
    faces: Option<(wgpu::TextureView, wgpu::TextureView, u32)>,
    pub(super) made: u64,
    /// Group 0 of the capture and of the faces' reading, and their
    /// pipelines (without an environment and with).
    /// (The capture's without the sun's cascades and with, its pipelines
    /// by that and by environment.)
    capture_layouts: [wgpu::BindGroupLayout; 2],
    capture: [[Option<wgpu::RenderPipeline>; 2]; 2],
    read_layouts: [wgpu::BindGroupLayout; 2],
    read: [Option<wgpu::ComputePipeline>; 2],
}

/// Whether the device can update probes by drawing faces: it runs
/// compute and reads storage buffers where the faces are lit and where
/// their vertices are placed (each draw's meshes are read from a list).
pub(super) fn rasterises(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    limits.max_compute_invocations_per_workgroup >= 64
        && limits.max_storage_buffers_per_shader_stage >= 2
        && crate::caps::Caps::of(device).storage_in_vertex
}

fn texture(
    device: &wgpu::Device,
    rows: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 6 * FACE,
                height: rows * FACE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl Raster {
    pub(super) fn new(layouts: ([wgpu::BindGroupLayout; 2], [wgpu::BindGroupLayout; 2])) -> Self {
        Self {
            meshes: None,
            held: Vec::new(),
            seen: Vec::new(),
            revision: 0,
            rebuilds: 0,
            shapes: HashMap::new(),
            instances: None,
            items: Items::default(),
            plan: Plan::default(),
            kept: None,
            face_buffer: None,
            face_layout: None,
            face: None,
            buffers_made: 0,
            instance_bytes: 0,
            cluster_bytes: 0,
            plans: 0,
            faces: None,
            made: 0,
            capture_layouts: layouts.0,
            capture: [[None, None], [None, None]],
            read_layouts: layouts.1,
            read: [None, None],
        }
    }

    /// The faces' atlas made to hold `rows` probes' faces (`MOST` at
    /// most), where it holds fewer.
    pub(super) fn hold(&mut self, device: &wgpu::Device, rows: u32) {
        let rows = rows.clamp(1, MOST);
        if self.faces.as_ref().is_some_and(|(.., have)| *have >= rows) {
            return;
        }
        let faces = texture(
            device,
            rows,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "3D probe faces",
        );
        let depth = texture(
            device,
            rows,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            "3D probe faces' depth",
        );
        self.faces = Some((faces, depth, rows));
        self.made += 1;
    }
    /// The faces' atlas, for the reading's binding (`hold` first).
    pub(super) fn faces(&self) -> &wgpu::TextureView {
        &self.faces.as_ref().expect("held").0
    }
    pub(super) fn capture_layout(&self, cascaded: bool) -> &wgpu::BindGroupLayout {
        &self.capture_layouts[usize::from(cascaded)]
    }
    pub(super) fn read_layout(&self, lit: bool) -> &wgpu::BindGroupLayout {
        &self.read_layouts[usize::from(lit)]
    }

    /// The scene made again of `drawn`, the world's meshes as of `meshes`,
    /// where what the faces see of it changed (`revision` then moves on):
    /// new geometries uploaded, gone ones dropped, the instances written.
    pub(super) fn rebuild(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        drawn: &[Drawn],
        meshes: u64,
    ) {
        let first = self.meshes.is_none();
        self.meshes = Some(meshes);
        self.seen.clear();
        self.seen
            .extend(drawn.iter().filter(|d| traced(d)).map(held));
        if self.seen == self.held && !first {
            return;
        }
        std::mem::swap(&mut self.seen, &mut self.held);
        self.revision += 1;
        self.rebuilds += 1;
        let now = self.rebuilds;
        let mut instances: Vec<(u64, Instance)> = Vec::with_capacity(self.held.len());
        for (d, h) in drawn.iter().filter(|d| traced(d)).zip(&self.held) {
            let shape = self.shapes.entry(h.geometry).or_insert_with(|| {
                let buffer = |contents: &[u8], usage, label| {
                    wgpu::util::DeviceExt::create_buffer_init(
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage,
                        },
                    )
                };
                self.buffers_made += 2;
                let (indices, clusters) = clustered(d.geometry.vertices(), d.geometry.indices());
                self.cluster_bytes +=
                    (indices.len() * 4 + clusters.len() * std::mem::size_of::<Cluster>()) as u64;
                Shape {
                    vertices: buffer(
                        bytemuck::cast_slice(d.geometry.vertices()),
                        wgpu::BufferUsages::VERTEX,
                        "3D probe faces' vertices",
                    ),
                    indices: buffer(
                        bytemuck::cast_slice(&indices),
                        wgpu::BufferUsages::INDEX,
                        "3D probe faces' indices",
                    ),
                    clusters,
                    used: now,
                }
            });
            shape.used = now;
            // Normals turn by the inverse transpose: its rows are the
            // inverse's columns.
            let inverse = d.matrix.inverse().unwrap_or(crate::Mat4::IDENTITY).columns;
            let t = &h.transform;
            let m = &h.material;
            instances.push((
                h.geometry,
                Instance {
                    place: [
                        [t[0], t[1], t[2], t[3]],
                        [t[4], t[5], t[6], t[7]],
                        [t[8], t[9], t[10], t[11]],
                    ],
                    turn: std::array::from_fn(|r| {
                        [inverse[r][0], inverse[r][1], inverse[r][2], 0.]
                    }),
                    albedo: [
                        m[0] * (1. - m[3]),
                        m[1] * (1. - m[3]),
                        m[2] * (1. - m[3]),
                        0.,
                    ],
                    glow: [m[4], m[5], m[6], 0.],
                },
            ));
        }
        let shapes = &self.shapes;
        self.cluster_bytes -= shapes
            .values()
            .filter(|shape| shape.used != now)
            .map(|shape| {
                let triangles: u32 = shape.clusters.iter().map(|c| c.triangles).sum();
                u64::from(triangles) * 12
                    + (shape.clusters.len() * std::mem::size_of::<Cluster>()) as u64
            })
            .sum::<u64>();
        self.shapes.retain(|_, shape| shape.used == now);
        // Each mesh's clusters as spheres in the world, by geometry,
        // cluster and mesh.
        let mut order: Vec<(u64, u32, u32)> = Vec::new();
        for (i, (geometry, _)) in instances.iter().enumerate() {
            for c in 0..self.shapes[geometry].clusters.len() as u32 {
                order.push((*geometry, c, i as u32));
            }
        }
        order.sort_unstable();
        let items = &mut self.items;
        items.spheres.clear();
        items.instance.clear();
        items.geometry.clear();
        items.cluster.clear();
        items.triangles.clear();
        for (geometry, c, i) in order {
            let cluster = &self.shapes[&geometry].clusters[c as usize];
            let place = &instances[i as usize].1.place;
            let at = |row: &[f32; 4]| {
                row[0] * cluster.center[0]
                    + row[1] * cluster.center[1]
                    + row[2] * cluster.center[2]
                    + row[3]
            };
            let scale = (0..3)
                .map(|col| {
                    (place[0][col].powi(2) + place[1][col].powi(2) + place[2][col].powi(2)).sqrt()
                })
                .fold(0f32, f32::max);
            items.spheres.push([
                at(&place[0]),
                at(&place[1]),
                at(&place[2]),
                cluster.radius * scale,
            ]);
            items.instance.push(i);
            items.geometry.push(geometry);
            items.cluster.push(c);
            items.triangles.push(cluster.triangles);
        }
        let count = instances.len() as u32;
        if self
            .instances
            .as_ref()
            .is_none_or(|(_, have)| *have < count)
        {
            let capacity = count.max(16).next_power_of_two();
            self.instances = Some((
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D probe faces' instances"),
                    size: u64::from(capacity) * std::mem::size_of::<Instance>() as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                capacity,
            ));
            self.buffers_made += 1;
            self.face = None;
        }
        if count > 0 {
            let bytes: Vec<Instance> = instances.into_iter().map(|(_, i)| i).collect();
            let bytes: &[u8] = bytemuck::cast_slice(&bytes);
            queue.write_buffer(&self.instances.as_ref().unwrap().0, 0, bytes);
            self.instance_bytes += bytes.len() as u64;
        }
    }

    /// This frame's probes at `places` (in turn) culled within `cut` of
    /// each, as many as fit `budget` triangles (one at least): how many
    /// it draws faces for (`update` then draws them). The meshes each
    /// face's draws take are uploaded.
    pub(super) fn plan(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        places: &[[f32; 3]],
        cut: f32,
        budget: u64,
    ) -> u32 {
        plan(&self.items, places, cut, budget, &mut self.plan);
        self.plans += 1;
        let entries = self.plan.kept.len() as u32;
        if self.kept.as_ref().is_none_or(|(_, have)| *have < entries) {
            let capacity = entries.max(256).next_power_of_two();
            self.kept = Some((
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D probe faces' meshes"),
                    size: u64::from(capacity) * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                capacity,
            ));
            self.buffers_made += 1;
            self.face = None;
        }
        if entries > 0 {
            queue.write_buffer(
                &self.kept.as_ref().unwrap().0,
                0,
                bytemuck::cast_slice(&self.plan.kept),
            );
        }
        self.plan.probes
    }

    /// This frame's `count` probes' faces drawn as `plan` culled them
    /// (`capture` binding), then read along their rays into the rays
    /// buffer (`read` binding).
    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (capture, read): (&wgpu::BindGroup, &wgpu::BindGroup),
        (lit, cascaded, count, rays): (bool, bool, u32, u32),
    ) {
        let source = |lit, cascaded| super::probes::raster_source(lit, cascaded);
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // Each face's index at its own dynamic offset, the meshes and the
        // list a draw's meshes are read from.
        let face_layout = self.face_layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("3D probe face"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(16),
                        },
                        count: None,
                    },
                    storage(1),
                    storage(2),
                ],
            })
        });
        let face_buffer = self.face_buffer.get_or_insert_with(|| {
            let mut contents = vec![0u8; (u64::from(MOST) * 6 * FACE_STRIDE) as usize];
            for face in 0..MOST * 6 {
                let at = (u64::from(face) * FACE_STRIDE) as usize;
                contents[at..at + 4].copy_from_slice(&face.to_le_bytes());
            }
            wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("3D probe face"),
                    contents: &contents,
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        });
        let face_bind = match (&self.instances, &self.kept) {
            (Some((instances, _)), Some((kept, _))) => Some(&*self.face.get_or_insert_with(|| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D probe face"),
                    layout: face_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: face_buffer,
                                offset: 0,
                                size: wgpu::BufferSize::new(16),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: instances.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: kept.as_entire_binding(),
                        },
                    ],
                })
            })),
            _ => None,
        };
        let capture_layout = &self.capture_layouts[usize::from(cascaded)];
        let capture_pipeline = self.capture[usize::from(cascaded)][usize::from(lit)]
            .get_or_insert_with(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("3D probe faces"),
                    source: wgpu::ShaderSource::Wgsl(source(lit, cascaded).into()),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("3D probe faces"),
                    bind_group_layouts: &[Some(capture_layout), Some(&*face_layout)],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("3D probe faces"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("capture_vertex"),
                        compilation_options: Default::default(),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<crate::Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                        }],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("capture_fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba32Float.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Less),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            });
        {
            let (faces, depth, _) = self.faces.as_ref().expect("held");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D probe faces"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: faces,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.,
                            g: 0.,
                            b: 0.,
                            a: EMPTY,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if let Some(face_bind) = face_bind
                && !self.plan.draws.is_empty()
            {
                pass.set_pipeline(capture_pipeline);
                pass.set_bind_group(0, capture, &[]);
                let (mut face, mut geometry) = (u32::MAX, None);
                for draw in &self.plan.draws {
                    if draw.face >= count * 6 {
                        break;
                    }
                    if draw.face != face {
                        face = draw.face;
                        let (x, y) = ((face % 6) * FACE, (face / 6) * FACE);
                        pass.set_viewport(x as f32, y as f32, FACE as f32, FACE as f32, 0., 1.);
                        pass.set_scissor_rect(x, y, FACE, FACE);
                        pass.set_bind_group(
                            1,
                            face_bind,
                            &[(u64::from(face) * FACE_STRIDE) as u32],
                        );
                    }
                    let shape = &self.shapes[&draw.geometry];
                    if geometry != Some(draw.geometry) {
                        geometry = Some(draw.geometry);
                        pass.set_vertex_buffer(0, shape.vertices.slice(..));
                        pass.set_index_buffer(shape.indices.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    let cluster = &shape.clusters[draw.cluster as usize];
                    pass.draw_indexed(
                        cluster.first * 3..(cluster.first + cluster.triangles) * 3,
                        0,
                        draw.kept.clone(),
                    );
                }
            }
        }
        let read_layout = &self.read_layouts[usize::from(lit)];
        let read_pipeline = self.read[usize::from(lit)].get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D probe faces read"),
                source: wgpu::ShaderSource::Wgsl(source(lit, false).into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D probe faces read"),
                bind_group_layouts: &[Some(read_layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("3D probe faces read"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("faces_to_rays"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("3D probe faces read"),
            timestamp_writes: None,
        });
        pass.set_pipeline(read_pipeline);
        pass.set_bind_group(0, read, &[]);
        pass.dispatch_workgroups((count * rays).div_ceil(64), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid of `n` by `n` quads, 1 apart: `2 n²` triangles.
    fn grid(n: u32) -> (Vec<crate::Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        for z in 0..=n {
            for x in 0..=n {
                vertices.push(crate::Vertex {
                    position: [x as f32, 0., z as f32],
                    normal: [0., 1., 0.],
                    uv: [0., 0.],
                });
            }
        }
        let mut indices = Vec::new();
        for z in 0..n {
            for x in 0..n {
                let a = z * (n + 1) + x;
                let b = a + n + 1;
                indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        (vertices, indices)
    }

    #[test]
    fn a_big_geometry_is_cut_into_clusters_holding_each_triangle_once() {
        // 20,000 triangles: clusters of CLUSTER or fewer, in order, each
        // sphere holding its triangles, every triangle in one.
        let (vertices, indices) = grid(100);
        let (reordered, clusters) = clustered(&vertices, &indices);
        assert!(
            clusters.len() >= 20_000 / CLUSTER,
            "{} clusters",
            clusters.len()
        );
        let mut next = 0;
        for c in &clusters {
            assert!(c.triangles as usize <= CLUSTER && c.triangles > 0, "{c:?}");
            assert_eq!(c.first, next);
            next += c.triangles;
            for &i in &reordered[c.first as usize * 3..(c.first + c.triangles) as usize * 3] {
                let p = vertices[i as usize].position;
                let d = (0..3)
                    .map(|k| (p[k] - c.center[k]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!(d <= c.radius * 1.0001 + 1e-5, "{p:?} outside {c:?}");
            }
        }
        assert_eq!(next as usize, indices.len() / 3);
        let sorted = |t: &[u32]| {
            let mut v: Vec<[u32; 3]> = t.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(sorted(&reordered), sorted(&indices));
        // The same every time.
        assert_eq!(clustered(&vertices, &indices), (reordered, clusters));
        // A small one is one cluster, as it was.
        let (vertices, indices) = grid(10);
        let (reordered, clusters) = clustered(&vertices, &indices);
        assert_eq!(clusters.len(), 1);
        assert_eq!(reordered, indices);
    }

    /// Items: unit spheres at `centres`, mesh `i` each, its own geometry,
    /// `triangles` each.
    fn items(centres: &[[f32; 3]], triangles: u32) -> Items {
        let mut items = Items::default();
        for (i, c) in centres.iter().enumerate() {
            items.spheres.push([c[0], c[1], c[2], 1.]);
            items.instance.push(i as u32);
            items.geometry.push(i as u64);
            items.cluster.push(0);
            items.triangles.push(triangles);
        }
        items
    }

    #[test]
    fn a_face_draws_what_lies_in_its_frustum_within_the_cut() {
        // Ahead along +x (face 0) at 5, behind it at -5 (face 1), up at
        // 5 (face 2), and far along +x at 50.
        let items = items(
            &[[5., 0., 0.], [-5., 0., 0.], [0., 5., 0.], [50., 0., 0.]],
            10,
        );
        let mut out = Plan::default();
        plan(&items, &[[0.; 3]], 20., u64::MAX, &mut out);
        fn faces(out: &Plan, mesh: u32) -> Vec<u32> {
            out.draws
                .iter()
                .filter(|d| d.kept.clone().any(|k| out.kept[k as usize] == mesh))
                .map(|d| d.face)
                .collect()
        }
        assert_eq!(faces(&out, 0), [0]);
        assert_eq!(faces(&out, 1), [1]);
        assert_eq!(faces(&out, 2), [2]);
        assert!(
            faces(&out, 3).is_empty(),
            "past the cut: {:?}",
            faces(&out, 3)
        );
        assert_eq!(out.probes, 1);
        assert_eq!(out.triangles, 30);
        // Within the cut, it's drawn.
        plan(&items, &[[0.; 3]], 60., u64::MAX, &mut out);
        assert_eq!(faces(&out, 3), [0]);
        // A sphere the probe stands in shows in every face.
        let inside = super::tests::items(&[[0.2, 0., 0.]], 10);
        plan(&inside, &[[0.; 3]], 20., u64::MAX, &mut out);
        let mut seen: Vec<u32> = out.draws.iter().map(|d| d.face).collect();
        seen.dedup();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_frame_updates_as_many_probes_as_its_budget_holds_and_one_at_least() {
        // Each probe sees one mesh of 100 triangles in one face.
        let items = items(&[[5., 0., 0.]], 100);
        let places = [[0.; 3]; 10];
        let mut out = Plan::default();
        plan(&items, &places, 20., 350, &mut out);
        assert_eq!((out.probes, out.triangles), (3, 300));
        assert!(out.draws.iter().all(|d| d.face < 3 * 6));
        plan(&items, &places, 20., 0, &mut out);
        assert_eq!((out.probes, out.triangles), (1, 100));
        plan(&items, &places, 20., u64::MAX, &mut out);
        assert_eq!((out.probes, out.triangles), (10, 1000));
    }

    #[test]
    fn meshes_sharing_a_geometry_draw_together() {
        // Two meshes of one geometry ahead: one draw for both.
        let mut items = items(&[[5., 0., 0.], [6., 0., 0.]], 10);
        items.geometry = vec![7, 7];
        let mut out = Plan::default();
        plan(&items, &[[0.; 3]], 20., u64::MAX, &mut out);
        assert_eq!(out.draws.len(), 1, "{:?}", out.draws);
        assert_eq!(out.draws[0].kept, 0..2);
        assert_eq!(out.kept, [0, 1]);
    }
}
