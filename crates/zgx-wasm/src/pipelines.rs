//! Pipelines: made at once, or on a thread of their own
//! (`create-*-pipeline-async`, the guest told by `pipeline-ready`), with
//! WGSL's overridable constants and the stencil state.
use crate::{
    device::Scope,
    gpu::{
        ComputePipeline, NO_DEVICE, RenderPipeline, blend, compare, format, index, topology,
        vertex_format, writes,
    },
    runtime::{Guest, ToGuest, bindings::zgx::app::gpu as wit},
};
use std::sync::{Arc, Mutex, atomic::Ordering};
use wasmtime::component::Resource;
use zgx_gpu::wgpu;

/// Pipelines being made on other threads at once, at most.
const MAX_PENDING: usize = 16;

/// What a pipeline made on another thread becomes.
type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;
pub struct RenderPipelineFuture {
    id: u64,
    slot: Slot<wgpu::RenderPipeline>,
    taken: bool,
}
pub struct ComputePipelineFuture {
    id: u64,
    slot: Slot<wgpu::ComputePipeline>,
    taken: bool,
}
impl crate::gpu::Charged for RenderPipelineFuture {}
impl crate::gpu::Charged for ComputePipelineFuture {}

/// A render pipeline's descriptor with what it names held: made here, or
/// on another thread.
pub(crate) struct RenderParts {
    label: Option<String>,
    layout: Option<wgpu::PipelineLayout>,
    vertex: (wgpu::ShaderModule, Option<String>, Vec<(String, f64)>),
    buffers: Vec<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)>,
    primitive: wgpu::PrimitiveState,
    depth_stencil: Option<wgpu::DepthStencilState>,
    multisample: wgpu::MultisampleState,
    fragment: Option<Fragment>,
}
struct Fragment {
    module: wgpu::ShaderModule,
    entry: Option<String>,
    constants: Vec<(String, f64)>,
    targets: Vec<Option<wgpu::ColorTargetState>>,
}
impl RenderParts {
    pub(crate) fn create(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        let buffers: Vec<wgpu::VertexBufferLayout<'_>> = self
            .buffers
            .iter()
            .map(|(stride, step_mode, attributes)| wgpu::VertexBufferLayout {
                array_stride: *stride,
                step_mode: *step_mode,
                attributes,
            })
            .collect();
        let vertex_constants = constants(&self.vertex.2);
        let fragment_constants = self
            .fragment
            .as_ref()
            .map(|f| constants(&f.constants))
            .unwrap_or_default();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: self.label.as_deref(),
            layout: self.layout.as_ref(),
            vertex: wgpu::VertexState {
                module: &self.vertex.0,
                entry_point: self.vertex.1.as_deref(),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &vertex_constants,
                    ..Default::default()
                },
                buffers: &buffers,
            },
            primitive: self.primitive,
            depth_stencil: self.depth_stencil.clone(),
            multisample: self.multisample,
            fragment: self.fragment.as_ref().map(|f| wgpu::FragmentState {
                module: &f.module,
                entry_point: f.entry.as_deref(),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &fragment_constants,
                    ..Default::default()
                },
                targets: &f.targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}
pub(crate) struct ComputeParts {
    label: Option<String>,
    layout: Option<wgpu::PipelineLayout>,
    module: wgpu::ShaderModule,
    entry: Option<String>,
    constants: Vec<(String, f64)>,
}
impl ComputeParts {
    pub(crate) fn create(&self, device: &wgpu::Device) -> wgpu::ComputePipeline {
        let constants = constants(&self.constants);
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: self.label.as_deref(),
            layout: self.layout.as_ref(),
            module: &self.module,
            entry_point: self.entry.as_deref(),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
            cache: None,
        })
    }
}
fn constants(owned: &[(String, f64)]) -> Vec<(&str, f64)> {
    owned
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect()
}

impl Guest {
    pub(crate) fn render_parts(
        &self,
        d: &wit::RenderPipelineDescriptor,
    ) -> wasmtime::Result<RenderParts> {
        let layout = match &d.layout {
            Some(layout) => Some(self.table.get(layout)?.0.clone()),
            None => None,
        };
        let fragment = match &d.fragment {
            Some(f) => Some(Fragment {
                module: self.table.get(&f.module)?.0.clone(),
                entry: f.entry_point.clone(),
                constants: f.constants.clone(),
                targets: f
                    .targets
                    .iter()
                    .map(|t| {
                        t.as_ref().map(|t| wgpu::ColorTargetState {
                            format: format(t.format),
                            blend: t.blend.map(|b| wgpu::BlendState {
                                color: blend(b.color),
                                alpha: blend(b.alpha),
                            }),
                            write_mask: writes(t.write_mask),
                        })
                    })
                    .collect(),
            }),
            None => None,
        };
        Ok(RenderParts {
            label: d.label.clone(),
            layout,
            vertex: (
                self.table.get(&d.vertex.module)?.0.clone(),
                d.vertex.entry_point.clone(),
                d.vertex.constants.clone(),
            ),
            buffers: d
                .vertex
                .buffers
                .iter()
                .map(|b| {
                    (
                        b.array_stride,
                        match b.step_mode {
                            wit::VertexStepMode::Vertex => wgpu::VertexStepMode::Vertex,
                            wit::VertexStepMode::Instance => wgpu::VertexStepMode::Instance,
                        },
                        b.attributes
                            .iter()
                            .map(|a| wgpu::VertexAttribute {
                                format: vertex_format(a.format),
                                offset: a.offset,
                                shader_location: a.shader_location,
                            })
                            .collect(),
                    )
                })
                .collect(),
            primitive: wgpu::PrimitiveState {
                topology: topology(d.primitive.topology),
                strip_index_format: d.primitive.strip_index_format.map(index),
                front_face: match d.primitive.front_face {
                    wit::FrontFace::Ccw => wgpu::FrontFace::Ccw,
                    wit::FrontFace::Cw => wgpu::FrontFace::Cw,
                },
                cull_mode: match d.primitive.cull_mode {
                    wit::CullMode::None => None,
                    wit::CullMode::Front => Some(wgpu::Face::Front),
                    wit::CullMode::Back => Some(wgpu::Face::Back),
                },
                unclipped_depth: d.primitive.unclipped_depth,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: d.depth_stencil.as_ref().map(|s| wgpu::DepthStencilState {
                format: format(s.format),
                depth_write_enabled: Some(s.depth_write_enabled),
                depth_compare: Some(compare(s.depth_compare)),
                stencil: s.stencil.map(stencil).unwrap_or_default(),
                bias: wgpu::DepthBiasState {
                    constant: s.depth_bias,
                    slope_scale: s.depth_bias_slope_scale,
                    clamp: s.depth_bias_clamp,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: d.multisample.count,
                mask: d.multisample.mask,
                alpha_to_coverage_enabled: d.multisample.alpha_to_coverage_enabled,
            },
            fragment,
        })
    }
    pub(crate) fn compute_parts(
        &self,
        d: &wit::ComputePipelineDescriptor,
    ) -> wasmtime::Result<ComputeParts> {
        Ok(ComputeParts {
            label: d.label.clone(),
            layout: match &d.layout {
                Some(layout) => Some(self.table.get(layout)?.0.clone()),
                None => None,
            },
            module: self.table.get(&d.module)?.0.clone(),
            entry: d.entry_point.clone(),
            constants: d.constants.clone(),
        })
    }
    /// `make` on a thread of its own with the device, its errors captured
    /// there; `pipeline-ready` tells the guest when it's done.
    fn made_elsewhere<T: Send + 'static>(
        &mut self,
        make: impl FnOnce(&wgpu::Device) -> T + Send + 'static,
    ) -> Result<(u64, Slot<T>), String> {
        let device = self.gpu.device.as_ref().ok_or(NO_DEVICE)?.device.clone();
        if self.pending.load(Ordering::Acquire) >= MAX_PENDING {
            return Err(format!("{MAX_PENDING} pipelines are being made already"));
        }
        self.next_future += 1;
        let id = self.next_future;
        let slot: Slot<T> = Arc::default();
        let (filled, pending, tx) = (slot.clone(), self.pending.clone(), self.to_self.clone());
        pending.fetch_add(1, Ordering::AcqRel);
        let spawned = std::thread::Builder::new()
            .name("zgx-wasm pipeline".into())
            .spawn(move || {
                let scope = Scope::push(&device);
                let made = make(&device);
                let errors = scope.pop();
                *filled.lock().unwrap() = Some(match errors.into_iter().next() {
                    Some(error) => Err(error),
                    None => Ok(made),
                });
                pending.fetch_sub(1, Ordering::AcqRel);
                let _ = tx.send(ToGuest::PipelineReady(id));
            });
        if let Err(error) = spawned {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return Err(format!("no thread to make it on: {error}"));
        }
        Ok((id, slot))
    }
    pub(crate) fn render_pipeline_async(
        &mut self,
        d: wit::RenderPipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<RenderPipelineFuture>, String>> {
        let parts = self.render_parts(&d)?;
        match self.made_elsewhere(move |device| parts.create(device)) {
            Ok((id, slot)) => self.hold(RenderPipelineFuture {
                id,
                slot,
                taken: false,
            }),
            Err(why) => Ok(Err(why)),
        }
    }
    pub(crate) fn compute_pipeline_async(
        &mut self,
        d: wit::ComputePipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<ComputePipelineFuture>, String>> {
        let parts = self.compute_parts(&d)?;
        match self.made_elsewhere(move |device| parts.create(device)) {
            Ok((id, slot)) => self.hold(ComputePipelineFuture {
                id,
                slot,
                taken: false,
            }),
            Err(why) => Ok(Err(why)),
        }
    }
}

impl wit::HostRenderPipelineFuture for Guest {
    fn id(&mut self, this: Resource<RenderPipelineFuture>) -> wasmtime::Result<u64> {
        Ok(self.table.get(&this)?.id)
    }
    fn take(
        &mut self,
        this: Resource<RenderPipelineFuture>,
    ) -> wasmtime::Result<Option<Result<Resource<RenderPipeline>, String>>> {
        let future = self.table.get_mut(&this)?;
        if future.taken {
            return Ok(Some(Err("take: taken already".into())));
        }
        let Some(made) = future.slot.lock().unwrap().take() else {
            return Ok(None);
        };
        future.taken = true;
        Ok(Some(match made {
            Ok(pipeline) => self.hold(RenderPipeline(pipeline))?,
            Err(why) => Err(why),
        }))
    }
    fn drop(&mut self, rep: Resource<RenderPipelineFuture>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostComputePipelineFuture for Guest {
    fn id(&mut self, this: Resource<ComputePipelineFuture>) -> wasmtime::Result<u64> {
        Ok(self.table.get(&this)?.id)
    }
    fn take(
        &mut self,
        this: Resource<ComputePipelineFuture>,
    ) -> wasmtime::Result<Option<Result<Resource<ComputePipeline>, String>>> {
        let future = self.table.get_mut(&this)?;
        if future.taken {
            return Ok(Some(Err("take: taken already".into())));
        }
        let Some(made) = future.slot.lock().unwrap().take() else {
            return Ok(None);
        };
        future.taken = true;
        Ok(Some(match made {
            Ok(pipeline) => self.hold(ComputePipeline(pipeline))?,
            Err(why) => Err(why),
        }))
    }
    fn drop(&mut self, rep: Resource<ComputePipelineFuture>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

fn stencil(s: wit::StencilState) -> wgpu::StencilState {
    let face = |f: wit::StencilFaceState| wgpu::StencilFaceState {
        compare: compare(f.compare),
        fail_op: stencil_op(f.fail_op),
        depth_fail_op: stencil_op(f.depth_fail_op),
        pass_op: stencil_op(f.pass_op),
    };
    wgpu::StencilState {
        front: face(s.front),
        back: face(s.back),
        read_mask: s.read_mask,
        write_mask: s.write_mask,
    }
}
fn stencil_op(o: wit::StencilOperation) -> wgpu::StencilOperation {
    use wgpu::StencilOperation as S;
    match o {
        wit::StencilOperation::Keep => S::Keep,
        wit::StencilOperation::Zero => S::Zero,
        wit::StencilOperation::Replace => S::Replace,
        wit::StencilOperation::Invert => S::Invert,
        wit::StencilOperation::IncrementClamp => S::IncrementClamp,
        wit::StencilOperation::DecrementClamp => S::DecrementClamp,
        wit::StencilOperation::IncrementWrap => S::IncrementWrap,
        wit::StencilOperation::DecrementWrap => S::DecrementWrap,
    }
}
