//! The app's post passes (`World::add_post`) on the device: a pipeline a
//! pass (and target format), its parameters' buffer, and its bindings of
//! the chain's targets. HDR passes trade the scene back and forth with its
//! other half, ending in the scene (an odd count copies back once); display
//! passes trade what the display shows, the last writing the view's target.
//!
//! A pass's pipelines are made on the compile worker (`crate::worker`), off
//! the frame: the chain runs without it until they're in, then counts it.
use crate::world::{AppPass, PostPassId, PostStage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zgx_gpu::wgpu;

/// A pipeline's key: its pass, target format, and whether it encodes sRGB
/// itself.
type Key = (PostPassId, wgpu::TextureFormat, bool);
/// Where the worker leaves a pass's pipelines (in its keys' order), or why
/// the device refused them.
type Landing = Arc<Mutex<Option<Result<Vec<wgpu::RenderPipeline>, String>>>>;

/// A pass's frame (the prelude's `ZgxFrame`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Frame {
    time: f32,
    delta: f32,
    index: u32,
    pad: u32,
}

pub(in crate::render) struct Apps {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    frame: wgpu::Buffer,
    pipelines: HashMap<Key, wgpu::RenderPipeline>,
    /// Passes whose pipelines the worker is making: those keys, and where
    /// they land.
    pending: HashMap<PostPassId, (Vec<Key>, Landing)>,
    params: HashMap<PostPassId, wgpu::Buffer>,
    /// Passes the device refused (their shader or pipeline), with why: not
    /// run, not tried again.
    pub(super) failed: HashMap<PostPassId, String>,
    /// By pass and the texture it reads (0 scene, 1 its other half, 2 what
    /// the display shows, 3 its other half), for `compiled`.
    binds: HashMap<(PostPassId, usize), wgpu::BindGroup>,
    /// The scene's other half, for the copy back into the scene.
    copy_bind: Option<wgpu::BindGroup>,
    compiled: u64,
    /// The bytes of the targets its bindings hold: those of the compile
    /// they were made for (`compiled`).
    held: u64,
}

/// Which of the graph's views a pass reads or writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Scene,
    Scene2,
    Shown,
    Shown2,
}

impl Apps {
    #[cold]
    pub(in crate::render) fn new(device: &wgpu::Device) -> Self {
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D post, app"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(2),
                uniform(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D post, app"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            layout,
            pipeline_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("3D post, app"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            frame: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D post, app frame"),
                size: std::mem::size_of::<Frame>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pipelines: HashMap::new(),
            pending: HashMap::new(),
            params: HashMap::new(),
            failed: HashMap::new(),
            binds: HashMap::new(),
            copy_bind: None,
            compiled: 0,
            held: 0,
        }
    }

    /// Lets go of what holds the graph's targets.
    pub(in crate::render) fn rest(&mut self) {
        self.binds.clear();
        self.copy_bind = None;
        self.held = 0;
    }

    /// `passes` of `stage` run in turn: HDR passes on the scene, display
    /// passes on what the display shows, the last into `target` (of
    /// `format`, encoded by the pass where `encode`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &super::Graph,
        passes: &[AppPass],
        stage: PostStage,
        (target, format, encode): (&wgpu::TextureView, wgpu::TextureFormat, bool),
        (tick, copy): (crate::render::Tick, &wgpu::RenderPipeline),
        copy_layout: &wgpu::BindGroupLayout,
    ) {
        let Some(targets) = graph.targets.as_ref() else {
            return;
        };
        let view = |slot: Slot| match slot {
            Slot::Scene => targets.scene.as_ref(),
            Slot::Scene2 => targets.others[0].as_ref(),
            Slot::Shown => targets.others[1].as_ref(),
            Slot::Shown2 => targets.others[2].as_ref(),
        };
        if self.compiled != graph.compiled {
            self.rest();
            self.compiled = graph.compiled;
        }
        // The renderer's clock, as custom materials read it.
        let frame = Frame {
            time: tick.time,
            delta: tick.delta,
            index: tick.index,
            pad: 0,
        };
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
        // Those whose pipelines are in, as `ready` counted them: one the
        // device refused, or still being made, is left out.
        let mut ours: Vec<&AppPass> = Vec::new();
        for pass in passes.iter().filter(|p| p.stage == stage) {
            if self.ready(pass, (format, encode)) {
                self.write_params(device, queue, pass);
                ours.push(pass);
            }
        }
        let (a, b) = match stage {
            PostStage::Hdr => (Slot::Scene, Slot::Scene2),
            PostStage::Display => (Slot::Shown, Slot::Shown2),
        };
        for (at, pass) in ours.iter().enumerate() {
            let (from, to) = if at % 2 == 0 { (a, b) } else { (b, a) };
            let last = at + 1 == ours.len();
            // A display stage's last pass writes the target.
            let (into, format, encode) = match (stage, last) {
                (PostStage::Display, true) => (Some(target), format, encode),
                _ => (view(to), super::SCENE, false),
            };
            let (Some(input), Some(into)) = (view(from), into) else {
                return;
            };
            let key = (pass.id, from as usize);
            if !self.binds.contains_key(&key) {
                let made = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D post, app"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.frame.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.params[&pass.id].as_entire_binding(),
                        },
                    ],
                });
                self.binds.insert(key, made);
                self.held = graph.bytes();
            }
            draw(
                encoder,
                &self.pipelines[&(pass.id, format, encode)],
                &self.binds[&key],
                into,
            );
        }
        // An odd count of HDR passes ended in the scene's other half: back
        // into the scene, where bloom and the last pass read it.
        if stage == PostStage::Hdr
            && ours.len() % 2 == 1
            && let (Some(from), Some(into)) = (view(Slot::Scene2), view(Slot::Scene))
        {
            let sampler = &self.sampler;
            let source = self.copy_bind.get_or_insert_with(|| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D post, copy"),
                    layout: copy_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(from),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                })
            });
            draw(encoder, copy, source, into);
        }
    }

    /// The passes of each stage whose pipelines are in (HDR, display), for
    /// the frame's chain to count, with what the worker made since the last
    /// frame taken and what `passes` still lack asked of it; whether it is
    /// making any. The display's passes are made into `out` (the view's
    /// format, encoding sRGB itself) as well as the scene's.
    pub(in crate::render) fn ready_counts(
        &mut self,
        device: &wgpu::Device,
        passes: &[AppPass],
        out: (wgpu::TextureFormat, bool),
    ) -> ((u8, u8), bool) {
        self.retain(passes);
        self.collect();
        let mut counts = (0u8, 0u8);
        for pass in passes {
            if self.failed.contains_key(&pass.id) {
                continue;
            }
            let missing: Vec<Key> = needs(pass, out)
                .filter(|key| !self.pipelines.contains_key(key))
                .collect();
            if missing.is_empty() {
                let count = match pass.stage {
                    PostStage::Hdr => &mut counts.0,
                    PostStage::Display => &mut counts.1,
                };
                *count = count.saturating_add(1);
            } else if !self.pending.contains_key(&pass.id) {
                self.begin(device, pass, missing);
            }
        }
        (counts, !self.pending.is_empty())
    }

    /// The bytes its bindings hold of targets a compile since let go (none
    /// once it has run on `compiled`, or rested).
    pub(in crate::render) fn stale(&self, compiled: u64) -> u64 {
        if self.compiled == compiled {
            0
        } else {
            self.held
        }
    }

    /// Whether `pass`'s pipelines into `out` are in (and the device took
    /// them).
    fn ready(&self, pass: &AppPass, out: (wgpu::TextureFormat, bool)) -> bool {
        !self.failed.contains_key(&pass.id)
            && needs(pass, out).all(|key| self.pipelines.contains_key(&key))
    }

    /// `pass`'s pipelines of `keys` made on the compile worker, in an error
    /// scope; refused here if no worker can start.
    #[cold]
    fn begin(&mut self, device: &wgpu::Device, pass: &AppPass, keys: Vec<Key>) {
        let landing: Landing = Default::default();
        let (device, layout) = (device.clone(), self.pipeline_layout.clone());
        let (wgsl, made_keys, done) = (pass.wgsl.clone(), keys.clone(), landing.clone());
        let job = move || {
            let made = scoped(&device, || {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("3D post, app"),
                    source: wgpu::ShaderSource::Wgsl(wgsl.to_string().into()),
                });
                made_keys
                    .iter()
                    .map(|&(_, format, encode)| pipeline(&device, &layout, &module, format, encode))
                    .collect()
            });
            *done.lock().unwrap_or_else(|p| p.into_inner()) = Some(made);
        };
        match crate::worker::compile_off_the_frame(job) {
            Ok(()) => {
                self.pending.insert(pass.id, (keys, landing));
            }
            Err(error) => {
                let why = format!("no thread to compile it on: {error}");
                self.failed.insert(pass.id, why);
            }
        }
    }

    /// What the worker made since the last frame, put in place, or noted
    /// as refused (not tried again).
    fn collect(&mut self) {
        let landed: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(&id, (_, landing))| {
                let made = landing.lock().unwrap_or_else(|p| p.into_inner()).take()?;
                Some((id, made))
            })
            .collect();
        for (id, made) in landed {
            let Some((keys, _)) = self.pending.remove(&id) else {
                continue;
            };
            #[cfg(test)]
            let made = match tests::refused(id) {
                Some(error) => Err(error),
                None => made,
            };
            match made {
                Ok(pipelines) => self.pipelines.extend(keys.into_iter().zip(pipelines)),
                Err(error) => {
                    self.failed.insert(id, error);
                }
            }
        }
    }

    /// `pass`'s parameters' buffer (made once), its values written.
    fn write_params(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pass: &AppPass) {
        let bytes: Vec<f32> = pass.values.iter().flatten().copied().collect();
        let size = (bytes.len() as u64 * 4).max(16);
        let buffer = self.params.entry(pass.id).or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D post, app params"),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        if !bytes.is_empty() {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&bytes));
        }
    }

    /// Lets go of what passes no longer in `passes` had.
    pub(super) fn retain(&mut self, passes: &[AppPass]) {
        let live = |id: &PostPassId| passes.iter().any(|p| p.id == *id);
        self.pipelines.retain(|(id, ..), _| live(id));
        self.pending.retain(|id, _| live(id));
        self.params.retain(|id, _| live(id));
        self.failed.retain(|id, _| live(id));
        self.binds.retain(|(id, _), _| live(id));
    }
}

/// The pipelines `pass` draws with into `out`: into the scene's format, and
/// a display pass into the view's too (its last writes the target).
fn needs(
    pass: &AppPass,
    (format, encode): (wgpu::TextureFormat, bool),
) -> impl Iterator<Item = Key> {
    let display = (pass.stage == PostStage::Display).then_some((pass.id, format, encode));
    std::iter::once((pass.id, super::SCENE, false)).chain(display)
}

/// A pass's pipeline of `module` into `format` (encoding sRGB itself where
/// `encode`).
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    encode: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D post, app"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("zgx_vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("zgx_fragment"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("ZGX_ENCODE", f64::from(u8::from(encode)))],
                ..Default::default()
            },
            targets: &[Some(format.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// What `make` makes on `device`, or what the device refused of it (a
/// validation or internal error), caught rather than left to the device's
/// handler: an app's pass never stops a frame.
fn scoped<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let made = make();
    let errors = [now(internal.pop()), now(validation.pop())];
    match errors.into_iter().flatten().next() {
        None => Ok(made),
        Some(error) => Err(error.to_string()),
    }
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    into: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D post, app"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: into,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

/// `future`, which resolves at once (wgpu's error scopes on native).
fn now<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{Camera, Geometry, Material, Object, PostParam, PostPass, ToneMapping, World};
    use std::cell::{Cell, RefCell};
    use zgx::{compose::prelude::*, widgets::Ui};
    use zgx_gpu::GpuRenderer;

    thread_local! {
        /// The pass a test has the device refuse, and how often it was
        /// tried.
        static REFUSE: Cell<Option<PostPassId>> = const { Cell::new(None) };
        static TRIED: Cell<u32> = const { Cell::new(0) };
        /// The HDR passes the last frame's chain counted.
        pub(in crate::render) static COUNTED: RefCell<Option<u8>> = const { RefCell::new(None) };
        /// Whether the chain draws bloom by passes where it could dispatch.
        pub(in crate::render) static BLOOM_BY_PASSES: Cell<bool> = const { Cell::new(false) };
        /// Whether the chain takes the device to run no compute.
        pub(in crate::render) static WITHOUT_COMPUTE: Cell<bool> = const { Cell::new(false) };
    }
    /// A refusal, as a backend's would come, for the pass a test names.
    pub(super) fn refused(id: PostPassId) -> Option<String> {
        (REFUSE.with(Cell::get) == Some(id)).then(|| {
            TRIED.with(|t| t.set(t.get() + 1));
            "refused for the test".into()
        })
    }

    const SCALE: &str = "#zgx post 1
fn post(p: PostIn) -> vec4<f32> {
    return color(p.uv) * params.k;
}
";

    const SIZE: u32 = 32;

    /// A grey wall filling a window's view, mapped linearly.
    struct Frames {
        world: World,
        gpu: GpuRenderer,
        ui: Ui,
        step: u32,
    }
    impl Frames {
        fn new() -> Option<Self> {
            let world = World::new();
            let grey = Material {
                color: [0.2, 0.2, 0.2, 1.],
                unlit: true,
                ..Default::default()
            };
            world.add(Object::mesh(Geometry::cuboid(20., 20., 0.1), grey));
            world.set_camera(Camera::looking_at([0., 0., 4.], [0., 0., 0.]));
            world.set_tone_mapping(ToneMapping::Linear);
            let gpu = match GpuRenderer::new(SIZE, SIZE) {
                Ok(gpu) => gpu,
                Err(error) if std::env::var_os("ZGX_REQUIRE_GPU").is_some() => {
                    panic!("ZGX_REQUIRE_GPU, but no renderer: {error}")
                }
                Err(_) => return None,
            };
            let mut ui = Ui::new(SIZE as f32, SIZE as f32);
            ui.mount(gpu_view("World", world.view()).size(SIZE as f32, SIZE as f32));
            Some(Self {
                world,
                gpu,
                ui,
                step: 0,
            })
        }
        /// A frame (the camera nudged, for one to be drawn): its centre's red.
        fn centre(&mut self) -> u8 {
            self.step += 1;
            let z = 4. + self.step as f32 * 1e-4;
            self.world
                .set_camera(Camera::looking_at([0., 0., z], [0., 0., 0.]));
            self.ui.prepare_frame();
            let report = self.ui.scene.borrow_mut().flush();
            self.gpu
                .render(&self.ui.scene.borrow(), &report.damage)
                .unwrap();
            let pixels = self.gpu.readback().unwrap();
            pixels[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize]
        }
        /// Frames until nothing keeps the world drawing (its passes made):
        /// the last's centre, and how many.
        fn compiled(&mut self) -> (u8, u32) {
            for frames in 1..=2000 {
                let centre = self.centre();
                if !self.world.view().animating() {
                    return (centre, frames);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            panic!("still compiling");
        }
    }

    #[test]
    fn a_pass_the_device_refuses_is_left_out_and_not_tried_again() {
        let Some(mut frames) = Frames::new() else {
            return;
        };
        let world = frames.world.clone();
        let scale = || PostPass::new(SCALE).param("k", PostParam::Float);
        let double = world.add_post(scale()).unwrap();
        world.set_post_param(double, "k", 2.).unwrap();
        let (doubled, _) = frames.compiled();
        // A second, halving, refused: left out, the first's light alone.
        let halve = world.add_post(scale()).unwrap();
        world.set_post_param(halve, "k", 0.5).unwrap();
        REFUSE.with(|r| r.set(Some(halve)));
        let (centre, _) = frames.compiled();
        assert_eq!(centre, doubled);
        for _ in 0..3 {
            assert_eq!(frames.centre(), doubled);
        }
        assert_eq!(TRIED.with(Cell::get), 1, "tried once");
        assert_eq!(COUNTED.with(|c| *c.borrow()), Some(1), "counted out");
    }

    #[test]
    fn automatic_exposure_on_a_device_without_compute_lets_its_world_rest() {
        let Some(mut frames) = Frames::new() else {
            return;
        };
        WITHOUT_COMPUTE.with(|w| w.set(true));
        frames.world.set_post(crate::Post {
            auto_exposure: Some(crate::AutoExposure::default()),
            ..Default::default()
        });
        assert!(frames.world.view().animating(), "until a frame says");
        frames.centre();
        assert!(!frames.world.view().animating(), "the stops alone: at rest");
    }

    #[test]
    fn a_pass_is_made_off_the_frame_and_runs_from_the_frame_after() {
        let Some(mut frames) = Frames::new() else {
            return;
        };
        let world = frames.world.clone();
        let plain = frames.centre();
        let double = world
            .add_post(PostPass::new(SCALE).param("k", PostParam::Float))
            .unwrap();
        world.set_post_param(double, "k", 2.).unwrap();
        // The frame it's added in asks the worker and draws without it,
        // the world drawing on until it's in.
        assert_eq!(frames.centre(), plain, "not waited for");
        assert!(world.view().animating(), "drawing on while it's made");
        let (doubled, _) = frames.compiled();
        assert!(doubled > plain + 30, "{doubled} after {plain}");
        assert!(crate::worker::compile_threads() >= 1);
    }

    #[test]
    fn the_frame_a_display_pass_is_refused_shows_the_rest_of_the_chain() {
        let Some(mut frames) = Frames::new() else {
            return;
        };
        let world = frames.world.clone();
        let scale = || PostPass::new(SCALE).param("k", PostParam::Float);
        let gain = world.add_post(scale()).unwrap();
        world.set_post_param(gain, "k", 1.).unwrap();
        let (plain, _) = frames.compiled();
        // In the same frame: the HDR pass doubling, a display pass added
        // and refused. Each frame shows the doubling, the first too.
        world.set_post_param(gain, "k", 2.).unwrap();
        let shown = world.add_post(scale().stage(PostStage::Display)).unwrap();
        world.set_post_param(shown, "k", 0.).unwrap();
        REFUSE.with(|r| r.set(Some(shown)));
        let first = frames.centre();
        let (later, _) = frames.compiled();
        assert!(later > plain + 30, "{later} after {plain}");
        assert_eq!(first, later, "its first frame");
    }
}
