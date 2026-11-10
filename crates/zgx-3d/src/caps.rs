//! What a view's device can do (design-3d-aaa.md, section 3): read from the
//! device a world's frames are drawn with, the first time one is drawn and
//! again after the device changes, and sorted into the design's tiers. A
//! feature that needs more than the device has says so through it, once,
//! rather than failing silently.
use zgx_gpu::wgpu;

/// Which of the design's tiers a device is (design-3d-aaa.md, section 3).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Compute, storage buffers and textures in every stage, indirect draws:
    /// Metal, Direct3D 12, Vulkan. Every feature runs.
    A,
    /// GL: no compute, no storage buffers in vertex shaders, no indirect
    /// draws assumed. Compute passes are `Unsupported`; particles update on
    /// the CPU.
    B,
}

/// The graphics API a device draws with.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Metal (macOS, iOS).
    Metal,
    /// Direct3D 12.
    Dx12,
    /// Vulkan.
    Vulkan,
    /// OpenGL or OpenGL ES.
    Gl,
    /// wgpu's no-op backend: nothing is drawn (benches; a view on Direct3D
    /// 11 today).
    Noop,
    /// Another.
    Other,
}

/// Which block-compressed texture formats a device samples.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compressed {
    /// BC1 to BC7 (desktop GPUs, Apple silicon).
    pub bc: bool,
    /// ASTC (mobile GPUs, Apple silicon).
    pub astc: bool,
    /// ETC2 and EAC.
    pub etc2: bool,
}

/// What a world's device can do ([`World::caps`](crate::World::caps)).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// Its tier: what of zgx-3d runs on it.
    pub tier: Tier,
    /// The API it draws with.
    pub backend: Backend,
    /// Compute shaders run (`World::add_compute`).
    pub compute: bool,
    /// Storage buffers a shader stage may bind (compute and fragment).
    pub storage_buffers: u32,
    /// Vertex shaders read storage buffers too (a buffer as instance data,
    /// skins, particles drawn from their state).
    pub storage_in_vertex: bool,
    /// Storage textures a shader stage may bind.
    pub storage_textures: u32,
    /// Draws and dispatches take their counts from a buffer.
    pub indirect: bool,
    /// Indirect draws may start at an instance other than the first.
    pub indirect_first_instance: bool,
    /// GPU time can be measured between passes.
    pub timestamps: bool,
    /// And inside them.
    pub timestamps_in_passes: bool,
    /// Block-compressed formats it samples.
    pub compressed: Compressed,
    /// Shaders trace rays against acceleration structures (ray queries).
    pub ray_queries: bool,
    /// Shaders index arrays of textures (bindless).
    pub texture_arrays: bool,
    /// The largest buffer, in bytes.
    pub max_buffer: u64,
    /// The largest part of a storage buffer one binding reads, in bytes.
    pub max_storage_binding: u64,
    /// The widest and tallest 2D texture, in texels.
    pub max_texture: u32,
    /// Workgroups a dispatch may have along each axis.
    pub max_workgroups: u32,
    /// A workgroup's size along each axis, at most.
    pub max_workgroup_size: [u32; 3],
    /// Invocations a workgroup may have in all.
    pub max_invocations: u32,
    /// Bytes of `var<workgroup>` a workgroup may have.
    pub max_workgroup_memory: u32,
}

impl Caps {
    /// What `device` can do.
    pub(crate) fn of(device: &wgpu::Device) -> Self {
        Self::from_parts(
            device.adapter_info().backend,
            device.features(),
            &device.limits(),
        )
    }

    /// What a device of `backend` with `features` and `limits` can do.
    ///
    /// wgpu keeps what a device can't do below WebGPU's baseline (its
    /// downlevel flags) on the adapter, which a view's frame doesn't give;
    /// a device's limits are what was asked of it. GL's devices are asked
    /// for downlevel limits that claim compute whether or not it runs, so
    /// GL is taken as the design's tier B, as section 3 says: nothing beyond
    /// it assumed. The other backends have compute, storage in every stage
    /// and indirect draws wherever wgpu runs on them.
    pub(crate) fn from_parts(
        backend: wgpu::Backend,
        features: wgpu::Features,
        limits: &wgpu::Limits,
    ) -> Self {
        let backend = match backend {
            wgpu::Backend::Metal => Backend::Metal,
            wgpu::Backend::Dx12 => Backend::Dx12,
            wgpu::Backend::Vulkan => Backend::Vulkan,
            wgpu::Backend::Gl => Backend::Gl,
            wgpu::Backend::Noop => Backend::Noop,
            _ => Backend::Other,
        };
        let gl = backend == Backend::Gl;
        let compute = !gl
            && limits.max_compute_workgroups_per_dimension > 0
            && limits.max_compute_invocations_per_workgroup > 0;
        let storage_buffers = if gl {
            0
        } else {
            limits.max_storage_buffers_per_shader_stage
        };
        let storage_textures = if gl {
            0
        } else {
            limits.max_storage_textures_per_shader_stage
        };
        let indirect = !gl;
        let storage_in_vertex = storage_buffers > 0;
        let tier = if compute && storage_in_vertex && storage_textures > 0 && indirect {
            Tier::A
        } else {
            Tier::B
        };
        let has = |feature| features.contains(feature);
        Self {
            tier,
            backend,
            compute,
            storage_buffers,
            storage_in_vertex,
            storage_textures,
            indirect,
            indirect_first_instance: indirect && has(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            timestamps: has(wgpu::Features::TIMESTAMP_QUERY),
            timestamps_in_passes: has(wgpu::Features::TIMESTAMP_QUERY)
                && has(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            compressed: Compressed {
                bc: has(wgpu::Features::TEXTURE_COMPRESSION_BC),
                astc: has(wgpu::Features::TEXTURE_COMPRESSION_ASTC),
                etc2: has(wgpu::Features::TEXTURE_COMPRESSION_ETC2),
            },
            ray_queries: has(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
            texture_arrays: has(wgpu::Features::TEXTURE_BINDING_ARRAY),
            max_buffer: limits.max_buffer_size,
            max_storage_binding: limits.max_storage_buffer_binding_size,
            max_texture: limits.max_texture_dimension_2d,
            max_workgroups: if compute {
                limits.max_compute_workgroups_per_dimension
            } else {
                0
            },
            max_workgroup_size: if compute {
                [
                    limits.max_compute_workgroup_size_x,
                    limits.max_compute_workgroup_size_y,
                    limits.max_compute_workgroup_size_z,
                ]
            } else {
                [0; 3]
            },
            max_invocations: if compute {
                limits.max_compute_invocations_per_workgroup
            } else {
                0
            },
            max_workgroup_memory: if compute {
                limits.max_compute_workgroup_storage_size
            } else {
                0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use zgx_gpu::viewport::{Viewport, ViewportFrame};

    fn noop() -> (wgpu::Device, wgpu::Queue) {
        wgpu::Device::noop(&Default::default())
    }

    /// A frame of `painter` at 16 by 16 on `device`.
    fn draw(painter: &mut impl Viewport, device: &wgpu::Device, queue: &wgpu::Queue) {
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut frame =
            ViewportFrame::new(device, queue, &mut encoder, &view, format, (16, 16), 1.);
        painter.render(&mut frame);
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn a_world_knows_its_devices_caps_once_a_frame_is_drawn() {
        let (device, queue) = noop();
        let world = World::new();
        assert_eq!(world.caps(), None, "no device before a frame");
        let mut painter = world.viewport();
        draw(&mut painter, &device, &queue);
        let caps = world.caps().expect("read with the first frame");
        assert_eq!(caps.backend, Backend::Noop);
        assert_eq!(caps, Caps::of(&device));
        // Another device: read again with its first frame.
        painter.device_changed();
        let limits = wgpu::Limits {
            max_storage_textures_per_shader_stage: 1,
            ..Default::default()
        };
        let (other, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        });
        draw(&mut painter, &other, &queue);
        assert_eq!(world.caps().unwrap().storage_textures, 1);
    }

    #[test]
    fn webgpus_baseline_is_tier_a_and_gl_is_tier_b() {
        let a = Caps::from_parts(
            wgpu::Backend::Metal,
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TEXTURE_COMPRESSION_ASTC,
            &wgpu::Limits::default(),
        );
        assert_eq!(a.tier, Tier::A);
        assert!(a.compute && a.storage_in_vertex && a.indirect);
        assert!(a.timestamps && !a.timestamps_in_passes);
        assert!(a.compressed.astc && !a.compressed.bc);
        assert!(!a.indirect_first_instance && !a.ray_queries && !a.texture_arrays);
        assert_eq!(a.max_workgroup_size, [256, 256, 64]);
        // GL as zgx's shared device asks for it: downlevel limits, which
        // claim compute whether or not it runs.
        let limits = wgpu::Limits::downlevel_defaults();
        assert!(limits.max_compute_workgroups_per_dimension > 0);
        let b = Caps::from_parts(
            wgpu::Backend::Gl,
            wgpu::Features::INDIRECT_FIRST_INSTANCE,
            &limits,
        );
        assert_eq!(b.tier, Tier::B);
        assert!(!b.compute && !b.storage_in_vertex && !b.indirect && !b.indirect_first_instance);
        assert_eq!((b.max_workgroups, b.max_invocations), (0, 0));
        // WebGL's limits: no compute at all.
        let webgl = wgpu::Limits::downlevel_webgl2_defaults();
        let c = Caps::from_parts(wgpu::Backend::Vulkan, wgpu::Features::empty(), &webgl);
        assert_eq!(c.tier, Tier::B);
        assert!(!c.compute);
    }
}
