//! What a renderer keeps on the device: its pipelines, layouts, bindings,
//! buffers by geometry, uploaded maps and targets.
use super::{
    Variant,
    frame::{FrameUniforms, environment_layout, frame_binding},
    graph::Graph,
    materials::{Extent, map_layout, texture_sampler},
    pipelines::main_pipelines,
    shader::wgsl,
    shadows::{Casting, SHADOW_SIZE, depth_map, shadow_pipeline},
    variant::ByVariant,
};
use crate::{
    math::{Mat4, Vec3},
    texture::MAPS,
};
use std::collections::HashMap;
use zgx_gpu::wgpu;
pub(super) struct Buffers {
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    /// A skinned geometry's joints and weights, a vertex's.
    pub(super) skin: Option<wgpu::Buffer>,
    /// A morphing geometry's targets' deltas, bound.
    pub(super) morphs: Option<wgpu::BindGroup>,
    /// Its second texture coordinates (its first where it has none), made
    /// with the first frame drawing it placed.
    pub(super) second_uvs: Option<wgpu::Buffer>,
    /// Its vertices' colours, if it has any.
    pub(super) colors: Option<wgpu::Buffer>,
    pub(super) count: u32,
    pub(super) used: u64,
}
/// What a frame drew its meshes from: a frame where the world's meshes are
/// as they were (`Inner::meshes`), the sun the same and no mesh translucent
/// draws its runs, instances and shadow map again.
#[derive(Clone, Copy)]
pub(super) struct Kept {
    pub(super) meshes: u64,
    /// The sun casting shadows, if one does.
    pub(super) sun: Option<Vec3>,
    /// Its view onto the shadow map, and a map texel's width.
    pub(super) shadow: Option<(Mat4, f32)>,
    /// Some mesh is translucent: its order follows the camera.
    pub(super) translucent: bool,
}
pub(super) struct Gpu {
    pub(super) format: wgpu::TextureFormat,
    /// Opaque and translucent meshes', without textures: an untextured
    /// scene binds none (a bound texture costs Apple's driver 16 MB).
    pub(super) plain: [wgpu::RenderPipeline; 2],
    /// Meshes' of any other variant, made with the first frame drawing one.
    pub(super) variants: HashMap<Variant, [wgpu::RenderPipeline; 2], ByVariant>,
    /// Skinned, morphing, cut-out or cut-out and placed meshes' depth from
    /// the sun, by which of those, made with the first such shadow.
    pub(super) shadow_variants: HashMap<u8, wgpu::RenderPipeline, ByVariant>,
    pub(super) shadow_layout: wgpu::BindGroupLayout,
    /// Group 1 of a skinned untextured pipeline: nothing.
    pub(super) empty_layout: wgpu::BindGroupLayout,
    pub(super) empty_bind: wgpu::BindGroup,
    /// Skinned meshes' joint matrices and morphing meshes' weights, all of
    /// a frame's in one buffer.
    pub(super) palette_layout: wgpu::BindGroupLayout,
    /// Group 3 of a morphing pipeline: its geometry's deltas.
    pub(super) morph_layout: wgpu::BindGroupLayout,
    pub(super) palettes: Option<(wgpu::Buffer, usize, wgpu::BindGroup)>,
    /// Whether the device reads storage buffers in vertex shaders: without
    /// (some GL), skinned and morphing meshes draw as bound.
    pub(super) skinning: bool,
    pub(super) uniforms: wgpu::Buffer,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) bind: wgpu::BindGroup,
    /// Depth from the sun, and its pass's frame (the uniforms alone: the
    /// map can't be read while drawn).
    pub(super) shadow: wgpu::RenderPipeline,
    pub(super) shadow_bind: wgpu::BindGroup,
    pub(super) sampler: wgpu::Sampler,
    /// The shadow map while shadows are on; a 1 by 1 stand-in otherwise.
    pub(super) shadow_map: Option<wgpu::TextureView>,
    pub(super) stand_in: wgpu::TextureView,
    /// What the shadow map holds: the sun's view and each opaque mesh's
    /// geometry and place. A frame where only the camera moved keeps it.
    pub(super) shadow_drawn: Option<Casting>,
    /// The frame's binding with the environment's cube, made with the first
    /// frame lit by one: its layout and sampler, the cube by environment
    /// id, and the binding (made again as the shadow map or cube changes).
    pub(super) environment_layout: Option<(wgpu::BindGroupLayout, wgpu::Sampler)>,
    pub(super) environment: Option<(u64, wgpu::TextureView)>,
    pub(super) environment_bind: Option<wgpu::BindGroup>,
    /// Textures by id, with the frame that last drew each.
    pub(super) texture_layout: wgpu::BindGroupLayout,
    pub(super) texture_sampler: wgpu::Sampler,
    /// Uploaded textures by id and whether read as sRGB (a texture may be
    /// a colour map in one place, data in another).
    pub(super) views: HashMap<(u64, bool), (wgpu::TextureView, u64)>,
    /// Group 1 with placements as well, and samplers wrapping other than
    /// by repeating: made with the first mesh needing them.
    pub(super) placed_layout: Option<wgpu::BindGroupLayout>,
    /// Group 1 with placements and a coat's or transmission's numbers as
    /// well, for coated meshes: made with the first.
    pub(super) layered_layout: Option<wgpu::BindGroupLayout>,
    /// Group 1 with what is behind as well, for meshes light passes
    /// through: made with the first.
    pub(super) glass_layout: Option<wgpu::BindGroupLayout>,
    pub(super) wrapped_samplers: HashMap<[crate::Wrap; 2], wgpu::Sampler>,
    /// Each set of maps meshes draw with (`map_set`), bound.
    pub(super) bindings: HashMap<[u64; MAPS], (wgpu::BindGroup, u64)>,
    /// What a map a mesh hasn't got reads: white (its factors as they are)
    /// and a flat normal. Made with the first textured frame.
    pub(super) stand_ins: Option<[wgpu::TextureView; MAPS]>,
    pub(super) instances: Option<(wgpu::Buffer, usize)>,
    pub(super) geometries: HashMap<u64, Buffers>,
    /// The frame's passes and their targets, what is behind meshes light
    /// passes through among them.
    pub(super) graph: Graph,
    /// What the last frame drew its meshes from (`Kept`); `None` before the
    /// first and after the bindings were made again.
    pub(super) kept: Option<Kept>,
    /// Bytes the last frame wrote into buffers (uniforms, poses, instances).
    pub(super) written: u64,
    /// Samples a pixel (`World::set_samples`).
    pub(super) samples: u32,
    pub(super) frame: u64,
}

impl Gpu {
    /// The shadow map made while `on` (dropped while off), and the frame's
    /// binding following it.
    pub(super) fn shadows(&mut self, device: &wgpu::Device, on: bool) {
        if on == self.shadow_map.is_some() {
            return;
        }
        self.shadow_map = on.then(|| depth_map(device, SHADOW_SIZE, "3D shadow map"));
        self.shadow_drawn = None;
        let map = self.shadow_map.as_ref().unwrap_or(&self.stand_in);
        self.bind = frame_binding(
            device,
            &self.layout,
            &self.uniforms,
            map,
            &self.sampler,
            None,
        );
        self.environment_bind = None;
    }
    /// The environment `environment`'s cube uploaded (once each) and the
    /// frame's binding with it.
    pub(super) fn environment(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        environment: &crate::Environment,
    ) {
        let data = &environment.data;
        if self
            .environment
            .as_ref()
            .is_none_or(|(id, _)| *id != data.id)
        {
            let levels: Vec<&[u8]> = data
                .levels
                .iter()
                .map(|level| bytemuck::cast_slice(level.as_slice()))
                .collect();
            let cube = zgx_gpu::viewport::cube_with_levels(
                device,
                queue,
                crate::environment::SIDE,
                &levels,
                "3D environment",
            );
            let view = cube.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
            self.environment = Some((data.id, view));
            self.environment_bind = None;
        }
        let (layout, sampler) = self
            .environment_layout
            .get_or_insert_with(|| environment_layout(device));
        if self.environment_bind.is_none() {
            let map = self.shadow_map.as_ref().unwrap_or(&self.stand_in);
            let cube = self.environment.as_ref().map(|(_, view)| (view, &*sampler));
            self.environment_bind = Some(frame_binding(
                device,
                layout,
                &self.uniforms,
                map,
                &self.sampler,
                cube,
            ));
        }
    }
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D"),
            source: wgpu::ShaderSource::Wgsl(wgsl(Variant::default(), samples).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D frame"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D frame"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("3D shadow map"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let stand_in = depth_map(device, 1, "3D shadow stand-in");
        let bind = frame_binding(device, &layout, &uniforms, &stand_in, &sampler, None);
        let texture_layout = map_layout(device, Extent::Maps);
        let texture_sampler = texture_sampler(device, [crate::Wrap::Repeat; 2]);
        let plain = main_pipelines(
            device,
            (format, samples),
            &shader,
            &[Some(&layout)],
            Variant::default(),
        );
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D shadow frame"),
            entries: &[uniform_entry],
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D shadow frame"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let shadow = shadow_pipeline(device, &shader, &[Some(&shadow_layout)], Variant::default());
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D nothing"),
            entries: &[],
        });
        let empty_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D nothing"),
            layout: &empty_layout,
            entries: &[],
        });
        let palette_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D joints"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let morph_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D morph targets"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        Self {
            format,
            plain,
            variants: HashMap::default(),
            morph_layout,
            environment_layout: None,
            environment: None,
            environment_bind: None,
            shadow_variants: HashMap::default(),
            shadow_layout,
            empty_layout,
            empty_bind,
            palette_layout,
            palettes: None,
            skinning: device.limits().max_storage_buffers_per_shader_stage > 0,
            uniforms,
            layout,
            bind,
            shadow,
            shadow_bind,
            sampler,
            shadow_map: None,
            stand_in,
            shadow_drawn: None,
            texture_layout,
            texture_sampler,
            placed_layout: None,
            layered_layout: None,
            glass_layout: None,
            wrapped_samplers: HashMap::new(),
            views: HashMap::new(),
            bindings: HashMap::new(),
            stand_ins: None,
            instances: None,
            geometries: HashMap::new(),
            graph: Graph::new(format, samples),
            kept: None,
            written: 0,
            samples,
            frame: 0,
        }
    }
}
