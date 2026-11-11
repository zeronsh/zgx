//! The scene as rays see it (`gi` feature): its meshes' geometries as
//! bottom-level acceleration structures, built once each, and its meshes
//! as one top-level structure's instances, built again only as the
//! world's meshes change (`Inner::meshes`): a still world builds nothing.
//! Their vertices, indices and materials are kept in storage buffers of
//! their own for shaders to light what a ray hits (Metal gives a hit no
//! vertices of its own).
//!
//! Left out: skinned and morphing meshes (they'd need their poses
//! rebuilt into a structure a frame) and meshes light passes through.
//! Textures aren't read at a hit: a mesh's colour is its material's.
use crate::world::Drawn;
use std::collections::BTreeMap;
use zgx_gpu::wgpu::{self, util::DeviceExt};

/// A mesh as a ray finds it (`RayInstance` in `probes_trace.wgsl`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct RayInstance {
    /// Its colour, and how metal it is (a metal reflects no diffuse light).
    albedo: [f32; 4],
    emissive: [f32; 4],
    /// Its geometry's first vertex and first index in the scene's buffers.
    geometry: [u32; 4],
}

/// A geometry's structure, and where its vertices and indices start in the
/// scene's buffers.
struct Entry {
    blas: wgpu::Blas,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    first_vertex: u32,
    first_index: u32,
    /// Built: its structure needs no build again.
    built: bool,
    /// The last rebuild that used it.
    used: u64,
}

pub(super) struct Scene {
    /// The world's meshes it holds (`Inner::meshes`); `None` before any.
    pub(super) meshes: Option<u64>,
    /// By geometry id, so structures build in the same order every run.
    geometries: BTreeMap<u64, Entry>,
    /// Every geometry's `Vertex`es and indices, one after another.
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    pub(super) tlas: wgpu::Tlas,
    capacity: u32,
    /// Each instance's `RayInstance`.
    pub(super) instances: wgpu::Buffer,
    /// Counts the times its buffers or structure were made again: what
    /// binds them is made again with it.
    pub(super) generation: u64,
    rebuilds: u64,
    /// What rays see of each traced mesh as built (`Held`), and this
    /// frame's, to compare: a change to meshes rays don't see (a pose of a
    /// skinned one, glass moving) builds nothing.
    held: Vec<Held>,
    seen: Vec<Held>,
    /// Counts the times what rays see changed: the probes converge again.
    pub(super) revision: u64,
}

/// A traced mesh as rays see it: its geometry, its place (a TLAS
/// instance's rows) and its colour, metalness and glow.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Held {
    pub(super) geometry: u64,
    pub(super) transform: [f32; 12],
    pub(super) material: [f32; 8],
}
pub(super) fn held(d: &Drawn) -> Held {
    let c = &d.matrix.columns;
    let row = |r: usize| [c[0][r], c[1][r], c[2][r], c[3][r]];
    let [a, b, e] = [row(0), row(1), row(2)];
    let m = d.material;
    let [r, g, bl, _] = m.color;
    let (albedo, glow) = if m.unlit {
        ([0.; 3], [r, g, bl])
    } else {
        ([r, g, bl], m.emissive)
    };
    Held {
        geometry: d.geometry.id(),
        transform: [
            a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3], e[0], e[1], e[2], e[3],
        ],
        material: [
            albedo[0],
            albedo[1],
            albedo[2],
            m.metallic.clamp(0., 1.),
            glow[0],
            glow[1],
            glow[2],
            0.,
        ],
    }
}

/// Whether the device traces rays.
pub(super) fn ray_queries(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
}

/// Whether rays see `d`.
pub(super) fn traced(d: &Drawn) -> bool {
    d.palette.is_none()
        && d.morph.is_none()
        && d.extras.is_none_or(|e| e.kinds & crate::world::GLASS == 0)
}

fn storage(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    // A buffer may not be empty, and is bound whole: a word at least.
    let bytes = if bytes.is_empty() {
        &[0; 16][..]
    } else {
        bytes
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE,
    })
}
fn tlas(device: &wgpu::Device, capacity: u32) -> wgpu::Tlas {
    device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("3D ray scene"),
        max_instances: capacity,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    })
}
fn instance_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D ray instances"),
        size: u64::from(capacity) * std::mem::size_of::<RayInstance>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Scene {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let capacity = 64;
        Self {
            meshes: None,
            geometries: BTreeMap::new(),
            vertices: storage(device, "3D ray vertices", &[]),
            indices: storage(device, "3D ray indices", &[]),
            tlas: tlas(device, capacity),
            capacity,
            instances: instance_buffer(device, capacity),
            generation: 0,
            rebuilds: 0,
            held: Vec::new(),
            seen: Vec::new(),
            revision: 0,
        }
    }

    /// The scene made again of `drawn`, the world's meshes as of `meshes`:
    /// new geometries' structures and the instances built into `encoder`,
    /// where what rays see of them changed (`revision` then moves on).
    pub(super) fn rebuild(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
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
        // Geometries: when one is new, the buffers are made again holding
        // all those in use (structures already built stay: a structure
        // keeps its own copy).
        let mut fresh = false;
        for d in drawn.iter().filter(|d| traced(d)) {
            match self.geometries.get_mut(&d.geometry.id()) {
                Some(entry) => entry.used = now,
                None => {
                    fresh = true;
                    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
                        vertex_format: wgpu::VertexFormat::Float32x3,
                        vertex_count: d.geometry.vertices().len() as u32,
                        index_format: Some(wgpu::IndexFormat::Uint32),
                        index_count: Some(d.geometry.indices().len() as u32),
                        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
                    };
                    let blas = device.create_blas(
                        &wgpu::CreateBlasDescriptor {
                            label: Some("3D ray geometry"),
                            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                        },
                        wgpu::BlasGeometrySizeDescriptors::Triangles {
                            descriptors: vec![size.clone()],
                        },
                    );
                    self.geometries.insert(
                        d.geometry.id(),
                        Entry {
                            blas,
                            size,
                            first_vertex: 0,
                            first_index: 0,
                            built: false,
                            used: now,
                        },
                    );
                }
            }
        }
        let gone = self.geometries.len();
        self.geometries.retain(|_, entry| entry.used == now);
        if fresh || gone != self.geometries.len() {
            let (mut vertices, mut indices) = (Vec::<u8>::new(), Vec::<u32>::new());
            let mut placed = std::collections::HashSet::new();
            for d in drawn.iter().filter(|d| traced(d)) {
                if !placed.insert(d.geometry.id()) {
                    continue;
                }
                let entry = self.geometries.get_mut(&d.geometry.id()).unwrap();
                entry.first_vertex = (vertices.len() / std::mem::size_of::<crate::Vertex>()) as u32;
                entry.first_index = indices.len() as u32;
                vertices.extend_from_slice(bytemuck::cast_slice(d.geometry.vertices()));
                indices.extend_from_slice(d.geometry.indices());
            }
            self.vertices = storage(device, "3D ray vertices", &vertices);
            self.indices = storage(device, "3D ray indices", bytemuck::cast_slice(&indices));
            self.generation += 1;
        }
        // Instances, and their materials.
        let count = self.held.len() as u32;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.tlas = tlas(device, self.capacity);
            self.instances = instance_buffer(device, self.capacity);
            self.generation += 1;
        }
        let mut records = Vec::with_capacity(count as usize);
        for (i, h) in self.held.iter().enumerate() {
            let entry = &self.geometries[&h.geometry];
            self.tlas[i] = Some(wgpu::TlasInstance::new(
                &entry.blas,
                h.transform,
                i as u32,
                0xff,
            ));
            let m = &h.material;
            records.push(RayInstance {
                albedo: [m[0], m[1], m[2], m[3]],
                emissive: [m[4], m[5], m[6], 0.],
                geometry: [entry.first_vertex, entry.first_index, 0, 0],
            });
        }
        for slot in count as usize..self.capacity as usize {
            if self.tlas[slot].is_none() {
                break;
            }
            self.tlas[slot] = None;
        }
        if !records.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&records));
        }
        let mut unbuilt: Vec<&mut Entry> =
            self.geometries.values_mut().filter(|e| !e.built).collect();
        let entries: Vec<wgpu::BlasBuildEntry> = unbuilt
            .iter()
            .map(|entry| wgpu::BlasBuildEntry {
                blas: &entry.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size: &entry.size,
                        vertex_buffer: &self.vertices,
                        first_vertex: entry.first_vertex,
                        vertex_stride: std::mem::size_of::<crate::Vertex>() as u64,
                        index_buffer: Some(&self.indices),
                        first_index: Some(entry.first_index),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), [&self.tlas]);
        drop(entries);
        for entry in &mut unbuilt {
            entry.built = true;
        }
    }
}
