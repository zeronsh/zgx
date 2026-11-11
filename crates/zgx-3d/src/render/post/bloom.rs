//! Bloom as dispatches of one compute pass, where the device runs compute
//! (`Chain::bloom_compute`): the same steps as the render path's passes, the
//! same taps (`bloom_maths.wgsl`), with no render pass a step. On Metal each
//! render pass a frame holds about 8 MB of driver memory at rest; ten steps
//! in one compute pass hold none of their own.
use super::super::graph::Graph;
use zgx_gpu::wgpu;

pub(super) struct Compute {
    layout: wgpu::BindGroupLayout,
    first: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
    up: wgpu::ComputePipeline,
    /// Each step's binding, in the order dispatched, for the compile they
    /// were made for: the first, the steps down, then up.
    binds: Option<(u64, Vec<wgpu::BindGroup>)>,
}

impl Compute {
    #[cold]
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D bloom, dispatched"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(include_str!("bloom_maths.wgsl"), include_str!("bloom.wgsl")).into(),
            ),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D bloom, dispatched"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: super::SCENE,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                texture(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D bloom, dispatched"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("3D bloom, dispatched"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            first: pipeline("first"),
            down: pipeline("down"),
            up: pipeline("up"),
            layout,
            binds: None,
        }
    }

    /// Lets go of the bindings holding the graph's targets.
    pub(super) fn rest(&mut self) {
        self.binds = None;
    }

    /// Bloom's steps from `scene`, as the render path's passes make them:
    /// down to its last, then up to its second (the last pass adds the
    /// second's tent to the first).
    pub(super) fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        (scene, sampler): (&wgpu::TextureView, &wgpu::Sampler),
    ) {
        let Some(targets) = graph.targets.as_ref() else {
            return;
        };
        let (down, up) = (&targets.bloom, &targets.bloom_up);
        if down.is_empty() {
            return;
        }
        if self
            .binds
            .as_ref()
            .is_none_or(|(c, _)| *c != graph.compiled)
        {
            let bind = |source: &wgpu::TextureView, out: &wgpu::TextureView, base| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D bloom step, dispatched"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(out),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(base),
                        },
                    ],
                })
            };
            let mut binds = vec![bind(scene, &down[0], scene)];
            for at in 1..down.len() {
                binds.push(bind(&down[at - 1], &down[at], &down[at - 1]));
            }
            // Step `at` up (bloom's level `at`, the up chain's `at - 1`):
            // its own down and the next one's tent, the last's down alone.
            for at in (1..down.len().saturating_sub(1)).rev() {
                let next = match at + 1 == down.len() - 1 {
                    true => &down[at + 1],
                    false => &up[at],
                };
                binds.push(bind(next, &up[at - 1], &down[at]));
            }
            self.binds = Some((graph.compiled, binds));
        }
        let Some((_, binds)) = &self.binds else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("3D bloom, dispatched"),
            timestamp_writes: None,
        });
        let half = graph.size();
        let size = |level: u32| {
            let (w, h) = ((half.0 / 2).max(1), (half.1 / 2).max(1));
            (
                ((w >> level).max(1)).div_ceil(8),
                ((h >> level).max(1)).div_ceil(8),
            )
        };
        let mut binds = binds.iter();
        for at in 0..down.len() as u32 {
            pass.set_pipeline(if at == 0 { &self.first } else { &self.down });
            pass.set_bind_group(0, binds.next().expect("a binding a step"), &[]);
            let (x, y) = size(at);
            pass.dispatch_workgroups(x, y, 1);
        }
        pass.set_pipeline(&self.up);
        for at in (1..down.len().saturating_sub(1) as u32).rev() {
            pass.set_bind_group(0, binds.next().expect("a binding a step"), &[]);
            let (x, y) = size(at);
            pass.dispatch_workgroups(x, y, 1);
        }
    }

    /// What the last pass reads as bloom's second step (its steps up added):
    /// the up chain's first, or with two steps, the second step down.
    pub(super) fn wider(graph: &Graph) -> Option<&wgpu::TextureView> {
        let targets = graph.targets.as_ref()?;
        targets.bloom_up.first().or(targets.bloom.get(1))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Bloom, Camera, Geometry, Material, Object, Post, ToneMapping, World};
    use zgx::{compose::prelude::*, widgets::Ui};
    use zgx_gpu::GpuRenderer;

    const SIZE: u32 = 128;

    /// A bright small square on black, bloomed, its frame drawn with bloom
    /// by passes or as dispatches.
    fn bloomed(by_passes: bool) -> Option<Vec<u8>> {
        super::super::apps::tests::BLOOM_BY_PASSES.with(|b| b.set(by_passes));
        let world = World::new();
        let hot = Material {
            color: [0., 0., 0., 1.],
            emissive: [6., 4., 2.],
            ..Default::default()
        };
        world.add(Object::mesh(Geometry::cuboid(0.4, 0.4, 0.1), hot));
        world.set_background_linear(Some([0., 0., 0., 1.]));
        world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
        world.set_tone_mapping(ToneMapping::Aces);
        world.set_post(Post {
            bloom: Some(Bloom { intensity: 0.6 }),
            ..Default::default()
        });
        let mut gpu = match GpuRenderer::new(SIZE, SIZE) {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
            }
            Err(_) => return None,
        };
        let mut ui = Ui::new(SIZE as f32, SIZE as f32);
        ui.mount(gpu_view("World", world.view()).size(SIZE as f32, SIZE as f32));
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
        Some(gpu.readback().unwrap())
    }

    #[test]
    fn bloom_as_dispatches_draws_as_bloom_by_passes() {
        let Some(passes) = bloomed(true) else {
            return;
        };
        let dispatched = bloomed(false).unwrap();
        // The glow reaches far from the square (bloom's widest steps).
        let at = |p: &[u8], x: u32, y: u32| p[((y * SIZE + x) * 4) as usize];
        assert!(
            at(&passes, SIZE / 2 + 30, SIZE / 2) > 3,
            "a glow to compare"
        );
        // The same frame: the same taps in the same format (the tests' passes
        // draw their steps in half floats too; in packed floats, three in 4
        // bytes, they're up to 4 levels off here).
        let most = passes
            .iter()
            .zip(&dispatched)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        // Where they part, for a device that rounds otherwise: how many
        // values, and the first's pixel.
        let differing = passes
            .iter()
            .zip(&dispatched)
            .filter(|(a, b)| a != b)
            .count();
        let first = passes.iter().zip(&dispatched).position(|(a, b)| a != b);
        let at = first.map(|i| (i / 4 % SIZE as usize, i / 4 / SIZE as usize, i % 4));
        assert_eq!(
            most, 0,
            "{most} levels apart: {differing} values differ, the first at (x, y, channel) {at:?}"
        );
    }
}
