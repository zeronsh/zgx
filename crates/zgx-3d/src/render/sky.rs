//! A world's sky (feature `media`, `World::set_sky`), drawn as a stock
//! `SceneDraw` after the opaque meshes: a grid over the frame at the far
//! plane, tested less-or-equal against their depth and writing none, so
//! only what no mesh covers runs it (and the picture behind glass gets it
//! too). The air's light is marched at the grid's corners and interpolated
//! (it changes slowly across the sky); the phases, the sun, the ground's
//! edge and the clouds are worked out per pixel. Its light is in the lights' units, into the HDR scene where the
//! post chain follows (exposed, bloomed, graded with the rest), else mapped
//! as the meshes are.
use super::scene_draw::{
    DrawPoint, FrameResources, SceneDraw, SceneFrame, SceneTarget, frame_wgsl,
};
use std::time::Instant;
use zgx_gpu::wgpu;

/// The sky's numbers as `sky.wgsl`'s `Sky`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkyUniform {
    pub(crate) rayleigh: [f32; 4],
    pub(crate) shape: [f32; 4],
    pub(crate) clouds: [f32; 4],
    pub(crate) drift: [f32; 4],
}

/// What it was made for the scene's targets (again as they change).
struct Made {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    sky_bind: wgpu::BindGroup,
}

/// The sky's draw: its numbers as the world set them, and how far its
/// clouds have drifted by frame time.
pub(crate) struct SkyDraw {
    /// Its numbers, the clouds' drift distance left at 0 (`update` fills it
    /// in by frame time).
    pub(crate) sky: SkyUniform,
    /// The clouds' speed, metres a second (0: still).
    pub(crate) speed: f32,
    /// The frame time the clouds started drifting from, and the last frame
    /// time it was written for: once a frame time, however many renders.
    start: Option<Instant>,
    written: Option<(Instant, SkyUniform)>,
    made: Option<Made>,
}
impl SkyDraw {
    pub(crate) fn new(sky: SkyUniform, speed: f32) -> Self {
        Self {
            sky,
            speed,
            start: None,
            written: None,
            made: None,
        }
    }
    /// Its numbers as set again (`World::set_sky`): written with the next
    /// frame.
    pub(crate) fn set(&mut self, sky: SkyUniform, speed: f32) {
        if speed == 0. {
            self.start = None;
        }
        self.sky = sky;
        self.speed = speed;
    }
}

impl SceneDraw for SkyDraw {
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: SceneTarget,
        frame: FrameResources<'_>,
    ) {
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D sky frame"),
            entries: &[uniform(0)],
        });
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D sky"),
            entries: &[uniform(0)],
        });
        let source = format!(
            "{}\n{}\n{}",
            frame_wgsl(),
            include_str!("shader/display.wgsl"),
            include_str!("sky.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D sky"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D sky"),
            bind_group_layouts: &[Some(&frame_layout), Some(&sky_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D sky"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // At the far plane, where no mesh drew: none written.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: target.depth,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: target.samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D sky"),
            size: std::mem::size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms, 0, bytemuck::bytes_of(&self.sky));
        let bind = |layout: &wgpu::BindGroupLayout, buffer: &wgpu::Buffer, label| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        };
        let frame_bind = bind(&frame_layout, frame.uniforms, "3D sky frame");
        let sky_bind = bind(&sky_layout, &uniforms, "3D sky");
        self.written = None;
        self.made = Some(Made {
            pipeline,
            uniforms,
            frame_bind,
            sky_bind,
        });
    }
    fn update(&mut self, queue: &wgpu::Queue, time: Instant) {
        let Some(made) = &self.made else {
            return;
        };
        // Once a frame time: a view in two boxes renders twice at one.
        if self.written.is_some_and(|(at, _)| at == time) {
            return;
        }
        let mut sky = self.sky;
        if self.speed != 0. {
            let start = *self.start.get_or_insert(time);
            sky.clouds[3] = time.saturating_duration_since(start).as_secs_f32() * self.speed;
        }
        if self.written.is_none_or(|(_, last)| last != sky) {
            queue.write_buffer(&made.uniforms, 0, bytemuck::bytes_of(&sky));
        }
        self.written = Some((time, sky));
    }
    fn draws_at(&self, point: DrawPoint) -> bool {
        point == DrawPoint::AfterOpaque && self.made.is_some()
    }
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _frame: &SceneFrame, _point: DrawPoint) {
        let Some(made) = &self.made else {
            return;
        };
        pass.set_pipeline(&made.pipeline);
        pass.set_bind_group(0, &made.frame_bind, &[]);
        pass.set_bind_group(1, &made.sky_bind, &[]);
        // `sky.wgsl`'s grid, COLUMNS by ROWS cells of two triangles.
        pass.draw(0..32 * 64 * 6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Renderer, testing};
    use super::*;
    use crate::{Clouds, Geometry, Light, Material, Object, Sky, World};
    use std::time::Duration;
    use zgx_gpu::viewport::{Viewport, ViewportFrame};

    /// A frame of `renderer` at 64 by 64: its passes.
    fn passes(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<String> {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
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
            ViewportFrame::new(device, queue, &mut encoder, &view, format, (64, 64), 1.);
        renderer.render(&mut frame);
        queue.submit([encoder.finish()]);
        let gpu = renderer.gpu.as_ref().unwrap();
        gpu.graph
            .passes()
            .iter()
            .map(|p| format!("{p:?}"))
            .collect()
    }

    fn world() -> World {
        let world = World::new();
        world.add(Object::mesh(
            Geometry::cuboid(1., 1., 1.),
            Material::rgb(0x808080),
        ));
        world.add_light(Light::Directional {
            direction: crate::vec3(-0.5, -1., -0.4),
            color: [1.; 3],
            intensity: 2.,
        });
        world
    }

    #[test]
    fn a_sky_draws_in_the_meshes_pass_and_a_world_without_one_has_no_draw() {
        let (device, queue) = testing::device_and_queue();
        let world = world();
        let mut renderer = world.renderer();
        let without = passes(&mut renderer, &device, &queue);
        assert!(renderer.gpu.as_ref().unwrap().draws_prepared.1.is_empty());
        world.set_sky(Some(Sky::new().clouds(Some(Clouds::new(0.5)))));
        assert_eq!(
            passes(&mut renderer, &device, &queue),
            without,
            "no pass of its own"
        );
        assert_eq!(renderer.gpu.as_ref().unwrap().draws_prepared.1.len(), 1);
        world.set_sky(None);
        assert_eq!(passes(&mut renderer, &device, &queue), without);
        assert_eq!(world.scene_draws(), 0, "its draw gone");
        assert!(world.sky().is_none());
    }

    #[test]
    fn its_shader_and_pipeline_are_valid() {
        let (device, queue) = testing::device_and_queue();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let mut draw = SkyDraw::new(Sky::new().uniform().0, 0.);
        for samples in [1, 4] {
            let target = SceneTarget {
                format: wgpu::TextureFormat::Rgba16Float,
                samples,
                depth: wgpu::TextureFormat::Depth32Float,
            };
            draw.prepare(
                &device,
                &queue,
                target,
                FrameResources { uniforms: &buffer },
            );
        }
        let error = testing::now(scope.pop());
        assert!(
            error.is_none(),
            "{}",
            error.map(|e| e.to_string()).unwrap_or_default()
        );
    }

    #[test]
    fn drifting_clouds_move_by_frame_time_once_however_many_renders() {
        let (device, queue) = testing::device_and_queue();
        let (sky, speed) = Sky::new()
            .clouds(Some(Clouds::new(0.5).drifting(10., [0., 1.])))
            .uniform();
        assert_eq!(speed, 10.);
        let mut draw = SkyDraw::new(sky, speed);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let target = SceneTarget {
            format: wgpu::TextureFormat::Rgba16Float,
            samples: 1,
            depth: wgpu::TextureFormat::Depth32Float,
        };
        draw.prepare(
            &device,
            &queue,
            target,
            FrameResources { uniforms: &buffer },
        );
        let start = Instant::now();
        draw.update(&queue, start);
        assert_eq!(draw.written.unwrap().1.clouds[3], 0.);
        let later = start + Duration::from_secs(2);
        draw.update(&queue, later);
        draw.update(&queue, later);
        let drift = draw.written.unwrap().1.clouds[3];
        assert!((drift - 20.).abs() < 1e-3, "{drift}");
        // Stopped: still, at their pattern as set.
        let (still, none) = Sky::new().clouds(Some(Clouds::new(0.5))).uniform();
        assert_eq!(none, 0.);
        draw.set(still, none);
        draw.update(&queue, later + Duration::from_secs(1));
        assert_eq!(draw.written.unwrap().1.clouds[3], 0.);
    }
}
