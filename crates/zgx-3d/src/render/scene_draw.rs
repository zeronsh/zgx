//! What draws into the scene beside the meshes, inside their pass: particle
//! emitters, decals, debug lines. A `SceneDraw` registered on a world
//! (`World::add_scene_draw`) draws into the same targets (the scene's colour,
//! multisampled, and its depth), at a point among the meshes it names, so
//! what it draws is depth-tested against them, multisampled, resolved and
//! mapped with the rest. A world with none pays nothing a frame.
use super::Run;
use crate::math::Mat4;
use std::{cell::RefCell, rc::Rc};
use zgx_gpu::wgpu;

/// Where among the meshes a `SceneDraw` draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawPoint {
    /// After the opaque meshes, before the translucent ones: what glass and
    /// translucent meshes should show behind them.
    AfterOpaque,
    /// After the translucent meshes too: what must show over glass.
    AfterTranslucent,
}

/// What a `SceneDraw` draws into: its pipelines are made for these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SceneTarget {
    /// The scene's colour format (linear light; a float format with the post
    /// chain).
    pub(crate) format: wgpu::TextureFormat,
    /// Samples a pixel.
    pub(crate) samples: u32,
    /// The depth's format (tested less-than, 0 near; not written by draws
    /// that don't need to).
    pub(crate) depth: wgpu::TextureFormat,
}

/// What a `SceneDraw` may build its pipelines against: the meshes' group 0
/// layout (the frame's binding, `shader/frame.wgsl`'s `Frame` at binding
/// 0, the sun's map and sampler beside), and the frame's uniform buffer
/// itself, to bind the camera in a layout of its own (group 0's layout has
/// a variant with the environment: a drawer binding its own is independent
/// of it).
#[derive(Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))] // Emitters, decals and debug draws will.
pub(crate) struct FrameResources<'a> {
    pub(crate) layout: &'a wgpu::BindGroupLayout,
    pub(crate) uniforms: &'a wgpu::Buffer,
}

/// The frame a `SceneDraw` draws in: the binding in group 0 as the meshes
/// set it, and the camera's view and projection apart (billboards, soft
/// particles) and as drawn.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SceneFrame<'a> {
    pub(crate) bind: &'a wgpu::BindGroup,
    pub(crate) view: Mat4,
    pub(crate) projection: Mat4,
    /// As the meshes are drawn this frame (with temporal antialiasing,
    /// moved by its jitter).
    pub(crate) view_projection: Mat4,
    /// The frame's size, in pixels.
    pub(crate) size: (u32, u32),
}

/// Something drawing into the scene with the meshes (see the module).
pub(crate) trait SceneDraw {
    /// Its pipelines made for `target`: called before its first draw and
    /// again only when the scene's formats or samples change.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: SceneTarget,
        frame: FrameResources<'_>,
    );
    /// Whether it draws at `point` this frame (before the pass: the meshes'
    /// bindings are restored after it only where it did).
    fn draws_at(&self, point: DrawPoint) -> bool;
    /// Its draws at `point`, into the meshes' pass, `frame.bind` set in
    /// group 0; it binds whatever else it needs.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &SceneFrame<'_>, point: DrawPoint);
}

/// The draws registered on a world, in the order added.
pub(crate) type SceneDraws = Vec<Rc<RefCell<dyn SceneDraw>>>;
/// The camera given to them: view, projection, and the view projection as
/// drawn.
pub(crate) type CameraParts = (Mat4, Mat4, Mat4);

/// The meshes' spans (`shown`, in the runs' order: opaque runs first) with
/// `draws` at their points between them. Out of the meshes' loop: a world
/// with no draws never comes here.
#[cold]
#[inline(never)]
pub(super) fn draw_with(
    pass: &mut wgpu::RenderPass<'_>,
    gpu: &super::Gpu,
    (runs, shown): (&[Run], &[(u32, std::ops::Range<u32>)]),
    frame: &SceneFrame<'_>,
    draws: &[Rc<RefCell<dyn SceneDraw>>],
    instances: &wgpu::Buffer,
) {
    let frame_bind = frame.bind;
    let opaque = shown.partition_point(|(r, _)| !runs[*r as usize].glass);
    let mut bound = super::draw::Bound::default();
    for spans in shown[..opaque].chunk_by(|a, b| a.0 == b.0) {
        let run = &runs[spans[0].0 as usize];
        super::draw::draw_run(pass, gpu, (run, spans), &mut bound);
    }
    let at = |pass: &mut wgpu::RenderPass<'_>, point| {
        let mut drew = false;
        for draw in draws {
            let draw = draw.borrow();
            if draw.draws_at(point) {
                draw.draw(pass, frame, point);
                drew = true;
            }
        }
        drew
    };
    if at(pass, DrawPoint::AfterOpaque) {
        // Its bindings in place of the meshes': set again for the rest.
        bound = super::draw::Bound::default();
        pass.set_bind_group(0, frame_bind, &[]);
        pass.set_vertex_buffer(1, instances.slice(..));
    }
    for spans in shown[opaque..].chunk_by(|a, b| a.0 == b.0) {
        let run = &runs[spans[0].0 as usize];
        super::draw::draw_run(pass, gpu, (run, spans), &mut bound);
    }
    at(pass, DrawPoint::AfterTranslucent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Geometry, Material, Object, World};
    use std::cell::Cell;
    use zgx::{compose::prelude::*, widgets::Ui};
    use zgx_gpu::GpuRenderer;

    const SIZE: u32 = 64;

    /// A colour over the whole frame far behind the meshes (clip depth
    /// 0.9999), tested against their depth, at `point`.
    struct Sheet {
        point: DrawPoint,
        color: [f64; 4],
        pipeline: Option<wgpu::RenderPipeline>,
        prepared: Rc<Cell<u32>>,
        /// A binding of its own of the frame's uniforms, made at prepare
        /// (that the buffer is one).
        own: Option<wgpu::BindGroup>,
    }
    impl SceneDraw for Sheet {
        fn prepare(
            &mut self,
            device: &wgpu::Device,
            _: &wgpu::Queue,
            target: SceneTarget,
            frame: FrameResources<'_>,
        ) {
            self.prepared.set(self.prepared.get() + 1);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sheet"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            self.own = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sheet"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.uniforms.as_entire_binding(),
                }],
            }));
            let _ = frame.layout;
            let [r, g, b, a] = self.color;
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sheet"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "@vertex fn v(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
                            let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
                            return vec4(uv * 2.0 - 1.0, 0.9999, 1.0);
                        }}
                        @fragment fn f() -> @location(0) vec4<f32> {{ return vec4({r:?}, {g:?}, {b:?}, {a:?}); }}"
                    )
                    .into(),
                ),
            });
            self.pipeline = Some(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("sheet"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("v"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("f"),
                        compilation_options: Default::default(),
                        targets: &[Some(target.format.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: target.depth,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Less),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: target.samples,
                        ..Default::default()
                    },
                    multiview_mask: None,
                    cache: None,
                }),
            );
        }
        fn draws_at(&self, point: DrawPoint) -> bool {
            point == self.point
        }
        fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &SceneFrame<'_>, _: DrawPoint) {
            // Given its frame: the camera apart and as drawn, the size.
            assert!(self.own.is_some());
            assert!(frame.view_projection == frame.projection * frame.view);
            assert_eq!(frame.size, (SIZE, SIZE));
            if let Some(pipeline) = &self.pipeline {
                pass.set_pipeline(pipeline);
                pass.draw(0..3, 0..1);
            }
        }
    }

    struct Window {
        ui: Ui,
        gpu: GpuRenderer,
    }
    impl Window {
        fn new(world: &World) -> Option<Self> {
            let gpu = match GpuRenderer::new(SIZE, SIZE) {
                Ok(gpu) => gpu,
                Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                    panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
                }
                Err(_) => return None,
            };
            let mut ui = Ui::new(SIZE as f32, SIZE as f32);
            ui.mount(gpu_view("World", world.view()).size(SIZE as f32, SIZE as f32));
            Some(Self { ui, gpu })
        }
        /// The centre's pixel and a corner's.
        fn draw(&mut self) -> ([u8; 4], [u8; 4]) {
            self.ui.prepare_frame();
            let report = self.ui.scene.borrow_mut().flush();
            self.gpu
                .render(&self.ui.scene.borrow(), &report.damage)
                .unwrap();
            let pixels = self.gpu.readback().unwrap();
            let at = |x: u32, y: u32| -> [u8; 4] {
                pixels[((y * SIZE + x) * 4) as usize..][..4]
                    .try_into()
                    .unwrap()
            };
            (at(SIZE / 2, SIZE / 2), at(2, 2))
        }
    }

    fn sheet(point: DrawPoint, color: [f64; 4]) -> (Rc<RefCell<dyn SceneDraw>>, Rc<Cell<u32>>) {
        let prepared = Rc::new(Cell::new(0));
        let draw = Rc::new(RefCell::new(Sheet {
            point,
            color,
            pipeline: None,
            prepared: prepared.clone(),
            own: None,
        }));
        (draw, prepared)
    }

    /// A world with a unlit cube of `material` filling the centre, near the
    /// camera (in front of the sheet's depth).
    fn world(material: Material) -> World {
        let world = World::new();
        world.add(Object::mesh(Geometry::cuboid(1., 1., 1.), material));
        world.set_camera(Camera::looking_at([0., 0., 2.], [0., 0., 0.]));
        world
    }

    #[test]
    fn a_scene_draw_is_hidden_by_meshes_in_front_and_prepared_once_for_its_targets() {
        let mut blue = Material::rgb(0x0000ff);
        blue.unlit = true;
        let world = world(blue);
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        let (red, prepared) = sheet(DrawPoint::AfterOpaque, [1., 0., 0., 1.]);
        world.add_scene_draw(red.clone());
        let (centre, corner) = window.draw();
        assert_eq!(centre, [0, 0, 255, 255], "the cube in front");
        assert_eq!(
            corner,
            [255, 0, 0, 255],
            "the sheet where nothing is nearer"
        );
        world.set_camera(Camera::looking_at([0., 0., 2.01], [0., 0., 0.]));
        window.draw();
        assert_eq!(prepared.get(), 1, "once for the same targets");
        world.set_samples(1);
        window.draw();
        assert_eq!(prepared.get(), 2, "again for others");
        world.remove_scene_draw(&red);
        let (_, corner) = window.draw();
        assert_eq!(corner[3], 0, "gone: the box's own background");
    }

    #[test]
    fn a_draw_after_translucent_meshes_shows_over_them_and_one_before_shows_through() {
        let mut glass = Material::rgb(0x0000ff);
        glass.unlit = true;
        glass.color[3] = 0.5;
        let shown = |point| {
            let world = world(glass);
            let mut window = Window::new(&world)?;
            world.add_scene_draw(sheet(point, [1., 0., 0., 1.]).0);
            Some(window.draw().0)
        };
        let Some(under) = shown(DrawPoint::AfterOpaque) else {
            return;
        };
        let over = shown(DrawPoint::AfterTranslucent).unwrap();
        // Before: the glass blended over the red; after: the red over the
        // glass (depth isn't written by translucent meshes).
        assert!(under[0] > 0 && under[2] > 0, "{under:?}");
        assert_eq!(over, [255, 0, 0, 255]);
    }
}
