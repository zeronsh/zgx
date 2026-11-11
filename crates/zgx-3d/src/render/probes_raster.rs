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
//! sun's shadow is the frame's map as last drawn (none under cascades).
//! A face is 16 by 16 texels: a ray reads the texel its direction falls in.
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

/// A geometry's vertices (`Vertex`) and indices, uploaded once.
struct Shape {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    /// The last rebuild that used it.
    used: u64,
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
    runs: Vec<(u64, std::ops::Range<u32>)>,
    /// Each face's index at its own offset (`FACE_STRIDE`), its layout and
    /// binding: a draw's face (its instance is its mesh).
    face: Option<(wgpu::BindGroupLayout, wgpu::BindGroup)>,
    /// Buffers made and instance bytes written, counted (for tests).
    pub(super) buffers_made: u64,
    pub(super) instance_bytes: u64,
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
/// compute and reads storage buffers where the faces are lit.
pub(super) fn rasterises(device: &wgpu::Device) -> bool {
    let limits = device.limits();
    limits.max_compute_invocations_per_workgroup >= 64
        && limits.max_storage_buffers_per_shader_stage >= 2
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
            runs: Vec::new(),
            face: None,
            buffers_made: 0,
            instance_bytes: 0,
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
                Shape {
                    vertices: buffer(
                        bytemuck::cast_slice(d.geometry.vertices()),
                        wgpu::BufferUsages::VERTEX,
                        "3D probe faces' vertices",
                    ),
                    indices: buffer(
                        bytemuck::cast_slice(d.geometry.indices()),
                        wgpu::BufferUsages::INDEX,
                        "3D probe faces' indices",
                    ),
                    count: d.geometry.indices().len() as u32,
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
        self.shapes.retain(|_, shape| shape.used == now);
        // By geometry, a run of instances each.
        instances.sort_by_key(|(geometry, _)| *geometry);
        self.runs.clear();
        for (i, (geometry, _)) in instances.iter().enumerate() {
            match self.runs.last_mut() {
                Some((g, range)) if g == geometry => range.end = i as u32 + 1,
                _ => self.runs.push((*geometry, i as u32..i as u32 + 1)),
            }
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
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                capacity,
            ));
            self.buffers_made += 1;
        }
        if count > 0 {
            let bytes: Vec<Instance> = instances.into_iter().map(|(_, i)| i).collect();
            let bytes: &[u8] = bytemuck::cast_slice(&bytes);
            queue.write_buffer(&self.instances.as_ref().unwrap().0, 0, bytes);
            self.instance_bytes += bytes.len() as u64;
        }
    }

    /// This frame's `count` probes' faces drawn (`capture` binding), then
    /// read along their rays into the rays buffer (`read` binding).
    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (capture, read): (&wgpu::BindGroup, &wgpu::BindGroup),
        (lit, cascaded, count, rays): (bool, bool, u32, u32),
    ) {
        let source = |lit, cascaded| super::probes::raster_source(lit, cascaded);
        // Each face's index, at its own dynamic offset: made once.
        let (face_layout, face_bind) = self.face.get_or_insert_with(|| {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("3D probe face"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                }],
            });
            let mut contents = vec![0u8; (u64::from(MOST) * 6 * FACE_STRIDE) as usize];
            for face in 0..MOST * 6 {
                let at = (u64::from(face) * FACE_STRIDE) as usize;
                contents[at..at + 4].copy_from_slice(&face.to_le_bytes());
            }
            let buffer = wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("3D probe face"),
                    contents: &contents,
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D probe face"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                }],
            });
            (layout, bind)
        });
        let capture_layout = &self.capture_layouts[usize::from(cascaded)];
        let capture_pipeline = self.capture[usize::from(cascaded)][usize::from(lit)]
            .get_or_insert_with(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("3D probe faces"),
                    source: wgpu::ShaderSource::Wgsl(source(lit, cascaded).into()),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("3D probe faces"),
                    bind_group_layouts: &[Some(capture_layout), Some(face_layout)],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("3D probe faces"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("capture_vertex"),
                        compilation_options: Default::default(),
                        buffers: &[
                            wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<crate::Vertex>() as u64,
                                step_mode: wgpu::VertexStepMode::Vertex,
                                attributes: &wgpu::vertex_attr_array![
                                    0 => Float32x3, 1 => Float32x3
                                ],
                            },
                            wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<Instance>() as u64,
                                step_mode: wgpu::VertexStepMode::Instance,
                                attributes: &wgpu::vertex_attr_array![
                                    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
                                    6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4
                                ],
                            },
                        ],
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
            if let Some((instances, _)) = &self.instances
                && !self.runs.is_empty()
            {
                pass.set_pipeline(capture_pipeline);
                pass.set_bind_group(0, capture, &[]);
                pass.set_vertex_buffer(1, instances.slice(..));
                for face in 0..count * 6 {
                    let (x, y) = ((face % 6) * FACE, (face / 6) * FACE);
                    pass.set_viewport(x as f32, y as f32, FACE as f32, FACE as f32, 0., 1.);
                    pass.set_scissor_rect(x, y, FACE, FACE);
                    pass.set_bind_group(1, &*face_bind, &[(u64::from(face) * FACE_STRIDE) as u32]);
                    for (geometry, run) in &self.runs {
                        let shape = &self.shapes[geometry];
                        pass.set_vertex_buffer(0, shape.vertices.slice(..));
                        pass.set_index_buffer(shape.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..shape.count, 0, run.clone());
                    }
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
