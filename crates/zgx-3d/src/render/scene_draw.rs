//! What draws into the scene beside the meshes, inside their pass: particle
//! emitters, decals, debug lines. A `SceneDraw` registered on a world
//! (`World::add_scene_draw`) draws into the same targets (the scene's colour,
//! multisampled, and its depth), at a point among the meshes it names, so
//! what it draws is depth-tested against them, multisampled, resolved and
//! mapped with the rest. A world with none pays nothing a frame.
//!
//! A draw sets its own group 0 (the meshes' has variants: with the
//! environment, the clustered lights, the probes), built from
//! `FrameResources::uniforms` in a layout of its own, `frame_wgsl()` its
//! WGSL; the meshes' bindings are set again after it.
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

/// What a `SceneDraw` builds its bindings with: the frame's uniform buffer,
/// `frame_wgsl()`'s `Frame` (a draw binds it in a group 0 of its own: the
/// meshes' layout varies with what the world lights by).
#[derive(Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))] // Emitters, decals and debug draws will.
pub(crate) struct FrameResources<'a> {
    pub(crate) uniforms: &'a wgpu::Buffer,
}

/// The WGSL of the frame's uniforms as `FrameResources::uniforms` holds
/// them: `struct Frame` and `@group(0) @binding(0) var<uniform> frame`
/// (the camera's `view_projection` as drawn, its position in `camera`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn frame_wgsl() -> &'static str {
    const FRAME: &str = include_str!("shader/frame.wgsl");
    const BINDING: &str = "var<uniform> frame: Frame;";
    let end = FRAME
        .find(BINDING)
        .map_or(FRAME.len(), |at| at + BINDING.len());
    &FRAME[..end]
}

/// The frame a `SceneDraw` draws in: the camera's view and projection apart
/// (billboards, soft particles) and as drawn.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SceneFrame {
    pub(crate) view: Mat4,
    pub(crate) projection: Mat4,
    /// As the meshes are drawn this frame (with temporal antialiasing,
    /// moved by its jitter).
    pub(crate) view_projection: Mat4,
    /// The frame's size, in pixels.
    pub(crate) size: (u32, u32),
    /// Whether this is the picture of what is behind meshes light passes
    /// through (`DrawPoint::AfterOpaque` only, before the scene's own).
    pub(crate) behind: bool,
}

/// Something drawing into the scene with the meshes (see the module).
pub(crate) trait SceneDraw {
    /// Its pipelines made for `target`: called before its first draw and
    /// again only when the scene's formats or samples change. (The picture
    /// behind meshes light passes through has the same targets.)
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
    /// Its data for the frame at `time` written, before the frame's passes:
    /// called once a frame time (a view drawn in two places renders twice
    /// at one, and is told once). Nothing by default.
    fn update(&mut self, _queue: &wgpu::Queue, _time: std::time::Instant) {}
    /// Its draws at `point`, into the meshes' pass: it sets its pipeline
    /// and every group it reads, group 0 its own (see the module). At
    /// `AfterOpaque`, twice a frame where something light passes through
    /// shows: `frame.behind`, then the scene.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &SceneFrame, point: DrawPoint);
}

/// The draws registered on a world, in the order added, each with the id
/// it was registered by (`World::add_scene_draw`): what it was prepared as
/// (an address can be a new draw's once one is dropped).
pub(crate) type SceneDraws = Vec<(u64, Rc<RefCell<dyn SceneDraw>>)>;
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
    (frame, frame_bind): (&SceneFrame, &wgpu::BindGroup),
    draws: &SceneDraws,
    instances: &wgpu::Buffer,
) {
    let opaque = shown.partition_point(|(r, _)| !runs[*r as usize].glass);
    let mut bound = super::draw::Bound::default();
    for spans in shown[..opaque].chunk_by(|a, b| a.0 == b.0) {
        let run = &runs[spans[0].0 as usize];
        super::draw::draw_run(pass, gpu, (run, spans), &mut bound);
    }
    let at = |pass: &mut wgpu::RenderPass<'_>, point| draw_at(pass, frame, draws, point);
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

/// `draws` at each point in a world with no meshes: nothing between them.
#[cold]
#[inline(never)]
pub(super) fn draw_alone(pass: &mut wgpu::RenderPass<'_>, frame: &SceneFrame, draws: &SceneDraws) {
    for point in [DrawPoint::AfterOpaque, DrawPoint::AfterTranslucent] {
        draw_at(pass, frame, draws, point);
    }
}

/// `draws`' draws at `point`, and whether any drew.
#[cold]
#[inline(never)]
pub(super) fn draw_at(
    pass: &mut wgpu::RenderPass<'_>,
    frame: &SceneFrame,
    draws: &SceneDraws,
    point: DrawPoint,
) -> bool {
    let mut drew = false;
    for (_, draw) in draws {
        let draw = draw.borrow();
        if draw.draws_at(point) {
            draw.draw(pass, frame, point);
            drew = true;
        }
    }
    drew
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
        /// Its draws into the picture behind transmitting meshes.
        behind: Rc<Cell<u32>>,
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
            // Its own group 0, of the frame's uniforms.
            self.own = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sheet"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.uniforms.as_entire_binding(),
                }],
            }));
            let [r, g, b, a] = self.color;
            let frame_wgsl = frame_wgsl();
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sheet"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{frame_wgsl}
                        @vertex fn v(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
                            let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
                            // Reading the frame (its camera's w is 1).
                            return vec4(uv * 2.0 - 1.0, 0.9999 * frame.camera.w, 1.0);
                        }}
                        @fragment fn f() -> @location(0) vec4<f32> {{ return vec4({r:?}, {g:?}, {b:?}, {a:?}); }}"
                    )
                    .into(),
                ),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sheet"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            self.pipeline = Some(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("sheet"),
                    layout: Some(&pipeline_layout),
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
        fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &SceneFrame, _: DrawPoint) {
            // Given its frame: the camera apart and as drawn, the size.
            assert!(frame.view_projection == frame.projection * frame.view);
            assert_eq!(frame.size, (SIZE, SIZE));
            self.behind.set(self.behind.get() + u32::from(frame.behind));
            if let (Some(pipeline), Some(own)) = (&self.pipeline, &self.own) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, own, &[]);
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

    type Draw = Rc<RefCell<dyn SceneDraw>>;
    fn sheet(point: DrawPoint, color: [f64; 4]) -> (Draw, Rc<Cell<u32>>) {
        let (draw, prepared, _) = sheet_behind(point, color);
        (draw, prepared)
    }
    /// A sheet, how often it was prepared, and how often it drew behind.
    fn sheet_behind(point: DrawPoint, color: [f64; 4]) -> (Draw, Rc<Cell<u32>>, Rc<Cell<u32>>) {
        let (prepared, behind) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let draw = Rc::new(RefCell::new(Sheet {
            point,
            color,
            pipeline: None,
            prepared: prepared.clone(),
            behind: behind.clone(),
            own: None,
        }));
        (draw, prepared, behind)
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

    #[test]
    fn a_draw_added_where_a_removed_one_was_is_prepared_and_drawn() {
        let mut blue = Material::rgb(0x0000ff);
        blue.unlit = true;
        let world = world(blue);
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        let (red, _) = sheet(DrawPoint::AfterOpaque, [1., 0., 0., 1.]);
        world.add_scene_draw(red.clone());
        window.draw();
        // Gone and freed, and another added between two frames: likely
        // where the first was.
        world.remove_scene_draw(&red);
        let was = Rc::as_ptr(&red) as *const ();
        drop(red);
        let (green, prepared) = sheet(DrawPoint::AfterOpaque, [0., 1., 0., 1.]);
        world.add_scene_draw(green.clone());
        let reused = Rc::as_ptr(&green) as *const () == was;
        let (_, corner) = window.draw();
        assert_eq!(
            prepared.get(),
            1,
            "the new draw prepared (address reused: {reused})"
        );
        assert_eq!(corner, [0, 255, 0, 255]);
    }

    #[test]
    fn a_draw_binds_its_own_group_0_in_a_world_with_an_environment() {
        // The meshes' group 0 has the environment's cube here; translucent
        // meshes after the draw bind it again.
        let mut glass = Material::rgb(0x0000ff);
        glass.color[3] = 0.5;
        let world = world(glass);
        world.set_environment(Some(crate::Environment::studio()));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        world.add_scene_draw(sheet(DrawPoint::AfterOpaque, [1., 0., 0., 1.]).0);
        let (centre, corner) = window.draw();
        assert_eq!(corner, [255, 0, 0, 255]);
        assert!(
            centre[0] > 0 && centre[2] > 0,
            "the glass over the sheet: {centre:?}"
        );
    }

    #[test]
    fn what_draws_among_opaque_meshes_shows_through_one_light_passes_through() {
        let mut white = Material::rgb(0xffffff);
        white.roughness = 0.;
        let world = World::new();
        let pane = world.add(Object::mesh(Geometry::cuboid(1., 1., 0.02), white));
        world.set_transmission(pane, Some(crate::Transmission::new(1.)));
        world.set_camera(Camera::looking_at([0., 0., 2.], [0., 0., 0.]));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        let (red, _, behind) = sheet_behind(DrawPoint::AfterOpaque, [1., 0., 0., 1.]);
        world.add_scene_draw(red);
        let (centre, _) = window.draw();
        assert_eq!(behind.get(), 1, "drawn into the picture behind the pane");
        assert!(
            centre[0] > 128 && centre[1] < 64,
            "red through the pane: {centre:?}"
        );
    }

    #[test]
    fn a_world_without_meshes_draws_its_scene_draws_and_an_empty_one_only_its_background() {
        // Emitters alone (no meshes): what's registered draws.
        let world = World::new();
        world.set_camera(Camera::looking_at([0., 0., 2.], [0., 0., 0.]));
        let Some(mut window) = Window::new(&world) else {
            return;
        };
        world.add_scene_draw(sheet(DrawPoint::AfterOpaque, [1., 0., 0., 1.]).0);
        let red = [255, 0, 0, 255];
        assert_eq!(
            window.draw(),
            (red, red),
            "drawn after the (no) opaque meshes"
        );
        // And after the (no) translucent ones, after it.
        world.add_scene_draw(sheet(DrawPoint::AfterTranslucent, [0., 1., 0., 1.]).0);
        let green = [0, 255, 0, 255];
        assert_eq!(window.draw(), (green, green), "then after the translucent");
        // Nothing at all: its background, as before.
        let empty = World::new();
        empty.set_background(Some(0x0000ff));
        let mut window = Window::new(&empty).unwrap();
        assert_eq!(window.draw(), ([0, 0, 255, 255], [0, 0, 255, 255]));
    }
}
