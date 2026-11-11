//! Automatic exposure on the device (`Post::auto_exposure`): a histogram of
//! the scene's log luminance in a compute pass, its middle's mean, and the
//! exposure eased toward it in a second, into a state the last pass and the
//! resolve read (`Chain::state`). The state is read back a frame or two
//! late, never waited for: the world stays unsettled while the exposure is
//! more than 1/64 stop from where it's going (design-3d-aaa.md, 4.4).
use crate::world::AutoExposure;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::Instant;
use zgx_gpu::wgpu;

/// The state's bytes: adapted, target, made, a pad (f32 each).
pub(super) const STATE: u64 = 16;

/// Where the state's copy is: none, copied, being mapped, mapped.
const IDLE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

/// The settings' uniform (the shader's `Settings`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Settings {
    range: [f32; 4],
    speed: [f32; 4],
}

pub(super) struct Exposure {
    histogram: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bins: wgpu::Buffer,
    settings: wgpu::Buffer,
    /// The binding of the graph's scene, for the compile it was made for.
    bind: Option<(u64, wgpu::BindGroup)>,
    read: wgpu::Buffer,
    copy: Arc<AtomicU8>,
    /// The last frame's time, for its step, and whether it had settled as
    /// it said then.
    last: Option<Instant>,
    answer: Option<bool>,
}

impl Exposure {
    #[cold]
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D exposure"),
            source: wgpu::ShaderSource::Wgsl(include_str!("exposure.wgsl").into()),
        });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D exposure"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("3D exposure"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let buffer = |size, usage, label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            histogram: pipeline("histogram"),
            adapt: pipeline("adapt"),
            layout,
            bins: buffer(256 * 4, wgpu::BufferUsages::STORAGE, "3D exposure bins"),
            settings: buffer(
                std::mem::size_of::<Settings>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                "3D exposure settings",
            ),
            bind: None,
            read: buffer(
                STATE,
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                "3D exposure, read",
            ),
            copy: Arc::new(AtomicU8::new(IDLE)),
            last: None,
            answer: None,
        }
    }

    /// Lets go of its binding of the scene.
    pub(super) fn rest(&mut self) {
        self.bind = None;
    }

    /// The scene's histogram and the exposure adapted toward it, into
    /// `state`, at the frame's `time`. Whether the exposure had settled,
    /// as of a frame or two before, where that came back.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &super::Graph,
        state: &wgpu::Buffer,
        settings: &AutoExposure,
        time: Instant,
    ) -> Option<bool> {
        // The same instant again (one view drawn twice a frame): adapted
        // already, its histogram the same, its answer the frame's.
        if self.last == Some(time) {
            return self.answer;
        }
        let settled = self.settled();
        self.answer = settled;
        let scene = graph.targets.as_ref()?.scene.as_ref()?;
        // A step of frame time; after a long gap, the frame's own light.
        let step = self
            .last
            .map(|last| time.saturating_duration_since(last).as_secs_f32());
        self.last = Some(time);
        let restart = step.is_none_or(|s| s > 0.25);
        let values = Settings {
            range: [
                settings.min,
                settings.max - settings.min,
                settings.low,
                settings.high,
            ],
            speed: [
                settings.up,
                settings.down,
                step.unwrap_or(0.),
                f32::from(u8::from(restart)),
            ],
        };
        queue.write_buffer(&self.settings, 0, bytemuck::bytes_of(&values));
        if self.bind.as_ref().is_none_or(|(c, _)| *c != graph.compiled) {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D exposure"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(scene),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.bins.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: state.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.settings.as_entire_binding(),
                    },
                ],
            });
            self.bind = Some((graph.compiled, bind));
        }
        let (_, bind) = self.bind.as_ref()?;
        let size = graph.targets.as_ref()?.size;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("3D exposure"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, bind, &[]);
            pass.set_pipeline(&self.histogram);
            pass.dispatch_workgroups(size.0.div_ceil(16), size.1.div_ceil(16), 1);
            pass.set_pipeline(&self.adapt);
            pass.dispatch_workgroups(1, 1, 1);
        }
        // The state copied to be read, unless a copy is still being read.
        if self.copy.load(Ordering::Acquire) == IDLE {
            encoder.copy_buffer_to_buffer(state, 0, &self.read, 0, STATE);
            self.copy.store(COPIED, Ordering::Release);
        }
        settled
    }

    /// Whether the last state read back had settled; the copy made since
    /// mapped, to be read with a later frame.
    fn settled(&mut self) -> Option<bool> {
        match self.copy.load(Ordering::Acquire) {
            COPIED => {
                let copy = self.copy.clone();
                copy.store(MAPPING, Ordering::Release);
                self.read
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        copy.store(
                            if result.is_ok() { MAPPED } else { IDLE },
                            Ordering::Release,
                        );
                    });
                None
            }
            MAPPED => {
                let [adapted, target, made, _]: [f32; 4] = {
                    let bytes = self.read.slice(..).get_mapped_range();
                    bytemuck::pod_read_unaligned(&bytes[..16])
                };
                self.read.unmap();
                self.copy.store(IDLE, Ordering::Release);
                Some(made > 0.5 && (adapted - target).abs() <= 1. / 64.)
            }
            _ => None,
        }
    }
}
