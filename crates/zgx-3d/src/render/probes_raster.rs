//! Probes updated without ray queries (`gi` feature): where the device has
//! compute but no ray queries (Direct3D 12 compiling with FXC, macOS before
//! 15, Vulkan without `VK_KHR_ray_query`). The scene rays would see
//! (`raytrace::traced`) is kept as one list of triangles in the world,
//! made again only as what rays see of it changes (`raytrace::Held`, as
//! the tracer's scene). Each of a frame's probes draws it into six faces
//! of a cube about itself, lit as a ray's hit is, and a compute pass reads
//! the faces along the probe's rays into the rays buffer the blend reads
//! (`probes_raster.wgsl`). From there it's the tracer's path.
//!
//! What differs from rays: point and spot lights don't cast shadows on
//! what the faces see (the meshes drawn directly don't either), and the
//! sun's shadow is the frame's map as last drawn (none under cascades).
//! A face is 16 by 16 texels: a ray reads the texel its direction falls in.
use super::raytrace::{Held, held, traced};
use crate::world::Drawn;
use zgx_gpu::wgpu;

/// A face's side, in texels (`FACE` in `probes_raster.wgsl`).
const FACE: u32 = 16;
/// The most probes a frame draws faces for: the faces' atlas grows to as
/// many rows of six faces as a frame draws, this many at most (a 96 by
/// 4,096 atlas, 8 MB with its depth).
pub(super) const MOST: u32 = 256;
/// A face texel nothing was drawn into (`EMPTY`).
const EMPTY: f64 = 1e30;

/// A soup vertex: place, normal, colour (less what's metal) and glow, in
/// the world.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Corner {
    position: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    glow: [f32; 3],
}

pub(super) struct Raster {
    /// The world's meshes it holds (`Inner::meshes`); `None` before any.
    pub(super) meshes: Option<u64>,
    held: Vec<Held>,
    seen: Vec<Held>,
    /// Counts the times what the faces see changed: the probes converge
    /// again.
    pub(super) revision: u64,
    soup: Option<(wgpu::Buffer, u32)>,
    /// The faces' atlas (a row of six faces a probe drawn a frame), its
    /// depth and rows, made with the first frame; and how many times it
    /// was made (what binds it is made again with it).
    faces: Option<(wgpu::TextureView, wgpu::TextureView, u32)>,
    pub(super) made: u64,
    /// Group 0 of the capture and of the faces' reading, and their
    /// pipelines (without an environment and with).
    capture_layout: wgpu::BindGroupLayout,
    capture: [Option<wgpu::RenderPipeline>; 2],
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
    pub(super) fn new(layouts: (wgpu::BindGroupLayout, [wgpu::BindGroupLayout; 2])) -> Self {
        Self {
            meshes: None,
            held: Vec::new(),
            seen: Vec::new(),
            revision: 0,
            soup: None,
            faces: None,
            made: 0,
            capture_layout: layouts.0,
            capture: [None, None],
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
    pub(super) fn capture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.capture_layout
    }
    pub(super) fn read_layout(&self, lit: bool) -> &wgpu::BindGroupLayout {
        &self.read_layouts[usize::from(lit)]
    }

    /// The scene made again of `drawn`, the world's meshes as of `meshes`,
    /// where what the faces see of it changed (`revision` then moves on).
    pub(super) fn rebuild(&mut self, device: &wgpu::Device, drawn: &[Drawn], meshes: u64) {
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
        let mut corners: Vec<Corner> = Vec::new();
        for d in drawn.iter().filter(|d| traced(d)) {
            let c = &d.matrix.columns;
            let place = |p: [f32; 3]| {
                std::array::from_fn(|r| c[0][r] * p[0] + c[1][r] * p[1] + c[2][r] * p[2] + c[3][r])
            };
            // Normals turn by the inverse transpose.
            let inverse = d.matrix.inverse().unwrap_or(crate::Mat4::IDENTITY).columns;
            let turn = |n: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|r| {
                    inverse[r][0] * n[0] + inverse[r][1] * n[1] + inverse[r][2] * n[2]
                })
            };
            let m = held(d).material;
            let albedo = [m[0] * (1. - m[3]), m[1] * (1. - m[3]), m[2] * (1. - m[3])];
            let glow = [m[4], m[5], m[6]];
            let vertices = d.geometry.vertices();
            for &i in d.geometry.indices() {
                let v = &vertices[i as usize];
                corners.push(Corner {
                    position: place(v.position),
                    normal: turn(v.normal),
                    albedo,
                    glow,
                });
            }
        }
        self.soup = (!corners.is_empty()).then(|| {
            let buffer = wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("3D probe faces' scene"),
                    contents: bytemuck::cast_slice(&corners),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            );
            (buffer, corners.len() as u32)
        });
    }

    /// This frame's `count` probes' faces drawn (`capture` binding), then
    /// read along their rays into the rays buffer (`read` binding).
    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (capture, read): (&wgpu::BindGroup, &wgpu::BindGroup),
        (lit, count, rays): (bool, u32, u32),
    ) {
        let source = |lit| super::probes::raster_source(lit);
        let capture_pipeline = self.capture[usize::from(lit)].get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D probe faces"),
                source: wgpu::ShaderSource::Wgsl(source(lit).into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D probe faces"),
                bind_group_layouts: &[Some(&self.capture_layout)],
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
                        array_stride: std::mem::size_of::<Corner>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3
                        ],
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
            if let Some((soup, corners)) = &self.soup {
                pass.set_pipeline(capture_pipeline);
                pass.set_bind_group(0, capture, &[]);
                pass.set_vertex_buffer(0, soup.slice(..));
                for face in 0..count * 6 {
                    let (x, y) = ((face % 6) * FACE, (face / 6) * FACE);
                    pass.set_viewport(x as f32, y as f32, FACE as f32, FACE as f32, 0., 1.);
                    pass.set_scissor_rect(x, y, FACE, FACE);
                    pass.draw(0..*corners, face..face + 1);
                }
            }
        }
        let read_layout = &self.read_layouts[usize::from(lit)];
        let read_pipeline = self.read[usize::from(lit)].get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D probe faces read"),
                source: wgpu::ShaderSource::Wgsl(source(lit).into()),
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
