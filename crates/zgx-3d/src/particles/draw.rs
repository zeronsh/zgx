//! Particles drawn with the meshes, in their pass (`render::SceneDraw`):
//! into the scene's colour and tested against its depth, multisampled,
//! resolved, and (with the post chain) exposed, bloomed and mapped with
//! them. A world's emitters share one drawer, registered with its first; a
//! painter's step (`gpu::Device`) hands it each frame's pools.
//!
//! Where they draw among the meshes: additive particles after the opaque
//! ones, so glass in front tints a glow behind it, and alpha-blended ones
//! after the translucent ones, so smoke in front of glass covers it. Until
//! order-independent transparency, neither is right everywhere: alpha
//! particles behind glass show over it, and glass in front of a glow dims
//! it as glass behind it would.
//!
//! Its group 0 is its own: the frame's uniforms at binding 0 (the camera as
//! the meshes are drawn, and their look), each emitter's numbers at 1. A
//! draw changes nothing (the renderer may draw a point twice a frame: into
//! what glass shows behind it, then the scene).
use super::{Blend, cpu::Particle};
use crate::render::{DrawPoint, FrameResources, SceneDraw, SceneFrame, SceneTarget, frame_wgsl};
use std::sync::{Arc, Mutex};
use zgx_gpu::wgpu;

/// One emitter's pool as a frame draws it.
pub(crate) struct Drawn {
    pub(crate) particles: wgpu::Buffer,
    pub(crate) slots: u32,
    /// Its group 0, made for the prepare `made`.
    pub(crate) bind: wgpu::BindGroup,
    pub(crate) made: u64,
    pub(crate) blend: Blend,
}

/// Alpha's and additive's pipelines for one target, made on the compile
/// worker.
type Pipelines = Arc<Mutex<Option<[wgpu::RenderPipeline; 2]>>>;

/// What a painter's renderer prepared the drawer with.
struct Prepared {
    painter: u64,
    /// Which prepare (each a number of its own): pools' bindings are made
    /// for one.
    made: u64,
    /// The device it prepared on: its identity (`ViewportFrame::device_id`).
    device: u64,
    /// Its group 0's layout, and the renderer's frame uniforms in it.
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    pipelines: Pipelines,
}

/// The painters a drawer keeps what it was prepared with for, at most (a
/// world's view and a few `World::viewport`s; a painter dropped lets its
/// go).
const MOST_PAINTERS: usize = 8;

/// A world's particles' drawer: one, registered with its first emitter,
/// whichever painters draw the world. Each painter's renderer prepares it
/// for its own targets and frame uniforms; the painter's step sets it as
/// `current` before the renderer draws, and hands it the pools.
#[derive(Default)]
pub(crate) struct ParticleDraw {
    /// Prepares so far.
    made: u64,
    prepared: Vec<Prepared>,
    /// The painter drawing this frame.
    /// The painter drawing this frame, and its device's identity.
    pub(crate) current: (u64, u64),
    /// Its pools this frame.
    pub(crate) drawn: Vec<Drawn>,
}

impl ParticleDraw {
    fn prepared(&self) -> Option<&Prepared> {
        self.prepared.iter().find(|p| p.painter == self.current.0)
    }
    /// Whether the current painter's pipelines are made.
    pub(crate) fn ready(&self) -> bool {
        self.prepared().is_some_and(|p| {
            p.pipelines
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
        })
    }
    /// What a pool's group 0 is made with for the current painter's
    /// renderer, if it prepared on `device` (where the pools are): which
    /// prepare, the layout and the frame's uniforms.
    pub(crate) fn binding_on(
        &self,
        device: u64,
    ) -> Option<(u64, wgpu::BindGroupLayout, wgpu::Buffer)> {
        let p = self.prepared().filter(|p| p.device == device)?;
        Some((p.made, p.layout.clone(), p.uniforms.clone()))
    }
    /// `painter`'s renderer gone, or its device: nothing of it kept.
    pub(crate) fn forget(&mut self, painter: u64) {
        self.prepared.retain(|p| p.painter != painter);
        if self.current.0 == painter {
            self.drawn.clear();
        }
    }
    /// The pools that draw at `point` this frame: made for the current
    /// painter's last prepare.
    fn at(&self, point: DrawPoint) -> impl Iterator<Item = &Drawn> {
        let blend = blend_at(point);
        let made = self.prepared().map(|p| p.made);
        self.drawn
            .iter()
            .filter(move |d| d.blend == blend && Some(d.made) == made)
    }
}

impl SceneDraw for ParticleDraw {
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        target: SceneTarget,
        frame: FrameResources<'_>,
    ) {
        // For the painter whose step came before this renderer's frame.
        let (painter, device_id) = self.current;
        self.forget(painter);
        self.made += 1;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D particles draw"),
            entries: &[uniform_entry(0), uniform_entry(1)],
        });
        let pipelines = Pipelines::default();
        if self.prepared.len() >= MOST_PAINTERS {
            self.prepared.remove(0);
        }
        self.prepared.push(Prepared {
            painter,
            made: self.made,
            device: device_id,
            layout: layout.clone(),
            uniforms: frame.uniforms.clone(),
            pipelines: pipelines.clone(),
        });
        let device = device.clone();
        let job = move || {
            let source = format!("{}\n{}", frame_wgsl(), include_str!("draw.wgsl"));
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D particles draw"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D particles draw"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let made = [Blend::Alpha, Blend::Additive]
                .map(|blend| pipeline(&device, &layout, &module, target, blend));
            *pipelines.lock().unwrap_or_else(|p| p.into_inner()) = Some(made);
        };
        if let Err(error) = crate::compute::compile_off_the_frame(job) {
            eprintln!("zgx-3d: particles' pipelines can't be made ({error}): none drawn");
        }
    }
    fn draws_at(&self, point: DrawPoint) -> bool {
        self.at(point).next().is_some() && self.ready()
    }
    /// Changes nothing: at `AfterOpaque` it draws twice where glass shows
    /// what's behind it (`frame.behind`, then the scene).
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _frame: &SceneFrame, point: DrawPoint) {
        let Some(prepared) = self.prepared() else {
            return;
        };
        let pipelines = prepared.pipelines.lock().unwrap_or_else(|p| p.into_inner());
        // Not made (yet) for these targets: nothing drawn.
        let Some(pipelines) = pipelines.as_ref() else {
            return;
        };
        pass.set_pipeline(&pipelines[blend_at(point) as usize]);
        for drawn in self.at(point) {
            pass.set_bind_group(0, &drawn.bind, &[]);
            pass.set_vertex_buffer(0, drawn.particles.slice(..));
            pass.draw(0..4, 0..drawn.slots);
        }
    }
}

/// Which blend draws at `point` (see the module).
fn blend_at(point: DrawPoint) -> Blend {
    match point {
        DrawPoint::AfterOpaque => Blend::Additive,
        DrawPoint::AfterTranslucent => Blend::Alpha,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    target: SceneTarget,
    blend: Blend,
) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32, 4 => Float32x4, 5 => Float32
    ];
    let component = match blend {
        Blend::Alpha => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        Blend::Additive => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D particles draw"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Particle>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &ATTRIBUTES,
            }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        // Tested against the meshes' depth, written by none: blended.
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
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.format,
                blend: Some(wgpu::BlendState {
                    color: component,
                    alpha: component,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
