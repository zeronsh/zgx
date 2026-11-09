//! `zgx:app/gpu`: WebGPU on the renderer's own device, for the guest.
//!
//! Each object the guest makes is a wgpu object of the host's device in the
//! store's resource table: a frame drawn into `current-texture` is sampled
//! by the element as it is, no copy. Everything is validated by wgpu, inside
//! error scopes of the guest's thread (errors are the guest's `err`s, or
//! lines of its log), and before wgpu where a mistake would panic it (a
//! buffer's range). The session's objects are capped in count and bytes.
use crate::{
    bundles::{QuerySet, RenderBundle},
    device::{Device, Scope},
    runtime::{Guest, bindings::zgx::app::gpu as wit},
};
use std::{borrow::Cow, num::NonZeroU64, time::Duration};
use wasmtime::component::Resource;
use zgx_gpu::wgpu;

/// The longest WGSL source a shader module takes.
const MAX_WGSL: usize = 4 << 20;
/// The most a buffer read back may be.
const MAX_READ: u64 = 64 << 20;

pub struct Buffer {
    pub(crate) buffer: wgpu::Buffer,
    usage: wit::BufferUsage,
}
pub struct Texture {
    pub(crate) texture: wgpu::Texture,
    /// Bytes charged to the session: none for a frame's texture, the host's.
    bytes: u64,
}
pub struct TextureView(wgpu::TextureView);
pub struct Sampler(wgpu::Sampler);
pub struct ShaderModule(pub(crate) wgpu::ShaderModule);
pub struct BindGroupLayout(wgpu::BindGroupLayout);
pub struct PipelineLayout(pub(crate) wgpu::PipelineLayout);
pub struct BindGroup(pub(crate) wgpu::BindGroup);
pub struct RenderPipeline(pub(crate) wgpu::RenderPipeline);
pub struct ComputePipeline(pub(crate) wgpu::ComputePipeline);
pub struct CommandBuffer(wgpu::CommandBuffer);
pub struct CommandEncoder(Option<wgpu::CommandEncoder>);
pub struct RenderPass(Option<wgpu::RenderPass<'static>>);
pub struct ComputePass(Option<wgpu::ComputePass<'static>>);

/// What an object costs the session.
pub(crate) trait Charged: Send + 'static {
    fn bytes(&self) -> u64 {
        0
    }
}
impl Charged for Buffer {
    fn bytes(&self) -> u64 {
        self.buffer.size()
    }
}
impl Charged for Texture {
    fn bytes(&self) -> u64 {
        self.bytes
    }
}
impl Charged for TextureView {}
impl Charged for Sampler {}
impl Charged for ShaderModule {}
impl Charged for BindGroupLayout {}
impl Charged for PipelineLayout {}
impl Charged for BindGroup {}
impl Charged for RenderPipeline {}
impl Charged for ComputePipeline {}
impl Charged for CommandBuffer {}
impl Charged for CommandEncoder {}
impl Charged for RenderPass {}
impl Charged for ComputePass {}

pub(crate) const NO_DEVICE: &str = "no GPU device yet: GPU objects are made in `frame`";

impl Guest {
    /// `f` on the device, its wgpu errors the `Err`.
    pub(crate) fn checked<T>(
        &self,
        f: impl FnOnce(&Device) -> Result<T, String>,
    ) -> Result<T, String> {
        let device = self.gpu.device.as_ref().ok_or(NO_DEVICE)?;
        let scope = Scope::push(&device.device);
        let value = f(device);
        let errors = scope.pop();
        let value = value?;
        match errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }
    /// An object for the guest, counted against the session's caps.
    pub(crate) fn hold<T: Charged>(
        &mut self,
        value: T,
    ) -> wasmtime::Result<Result<Resource<T>, String>> {
        let bytes = value.bytes();
        if self.gpu.objects >= self.gpu.max_objects {
            return Ok(Err(format!(
                "the session holds {} GPU objects, its most",
                self.gpu.max_objects
            )));
        }
        if self.gpu.bytes.saturating_add(bytes) > self.gpu.max_bytes {
            return Ok(Err(format!(
                "{bytes} more bytes would pass the session's {} of GPU memory",
                self.gpu.max_bytes
            )));
        }
        self.gpu.objects += 1;
        self.gpu.bytes += bytes;
        Ok(Ok(self.table.push(value)?))
    }
    pub(crate) fn release<T: Charged>(&mut self, handle: Resource<T>) -> wasmtime::Result<T> {
        let value = self.table.delete(handle)?;
        self.gpu.objects = self.gpu.objects.saturating_sub(1);
        self.gpu.bytes = self.gpu.bytes.saturating_sub(value.bytes());
        Ok(value)
    }
    /// Count `draws` and `workgroups` against the submission's caps: past
    /// them the command is left out and the submission refused.
    pub(crate) fn spend(&mut self, draws: u64, workgroups: u64, what: &str) -> bool {
        let gpu = &mut self.gpu;
        gpu.draws = gpu.draws.saturating_add(draws);
        gpu.workgroups = gpu.workgroups.saturating_add(workgroups);
        if gpu.draws > gpu.caps.draws || gpu.workgroups > gpu.caps.workgroups {
            if gpu.over.is_none() {
                gpu.over = Some(format!(
                    "{what}: past a submission's {} draws or {} workgroups",
                    gpu.caps.draws, gpu.caps.workgroups
                ));
            }
            return false;
        }
        true
    }
    /// A line of the log for what a method with no result got wrong.
    pub(crate) fn misuse(&self, what: &str, why: impl std::fmt::Display) {
        self.log.line(format!("gpu: {what}: {why}"));
    }
    pub(crate) fn buffer(&self, handle: &Resource<Buffer>) -> wasmtime::Result<wgpu::Buffer> {
        Ok(self.table.get(handle)?.buffer.clone())
    }
    fn texture_copy(
        &self,
        copy: &wit::TextureCopy,
    ) -> wasmtime::Result<(wgpu::Texture, u32, wgpu::Origin3d, wgpu::TextureAspect)> {
        Ok((
            self.table.get(&copy.texture)?.texture.clone(),
            copy.mip_level,
            wgpu::Origin3d {
                x: copy.origin.x,
                y: copy.origin.y,
                z: copy.origin.z,
            },
            aspect(copy.aspect),
        ))
    }
}

/// `offset..offset + size` (to its end if `size` is none) of `buffer`, or
/// why not: wgpu panics on an empty or overlong slice.
pub(crate) fn range(
    buffer: &wgpu::Buffer,
    offset: u64,
    size: Option<u64>,
) -> Result<std::ops::Range<u64>, String> {
    let length = buffer.size();
    let size = size.unwrap_or(length.saturating_sub(offset));
    match offset.checked_add(size) {
        Some(end) if size > 0 && end <= length => Ok(offset..end),
        _ => Err(format!("bytes {offset}..+{size} of a buffer of {length}")),
    }
}

impl wit::HostBuffer for Guest {
    fn size(&mut self, this: Resource<Buffer>) -> wasmtime::Result<u64> {
        Ok(self.table.get(&this)?.buffer.size())
    }
    fn usage(&mut self, this: Resource<Buffer>) -> wasmtime::Result<wit::BufferUsage> {
        Ok(self.table.get(&this)?.usage)
    }
    fn drop(&mut self, rep: Resource<Buffer>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostTexture for Guest {
    fn create_view(
        &mut self,
        this: Resource<Texture>,
        descriptor: Option<wit::TextureViewDescriptor>,
    ) -> wasmtime::Result<Result<Resource<TextureView>, String>> {
        let texture = self.table.get(&this)?.texture.clone();
        let made = self.checked(|_| {
            Ok(match &descriptor {
                None => texture.create_view(&Default::default()),
                Some(d) => texture.create_view(&wgpu::TextureViewDescriptor {
                    label: d.label.as_deref(),
                    format: d.format.map(format),
                    dimension: d.dimension.map(view_dimension),
                    usage: None,
                    aspect: aspect(d.aspect),
                    base_mip_level: d.base_mip_level,
                    mip_level_count: d.mip_level_count,
                    base_array_layer: d.base_array_layer,
                    array_layer_count: d.array_layer_count,
                }),
            })
        });
        match made {
            Ok(view) => self.hold(TextureView(view)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn width(&mut self, this: Resource<Texture>) -> wasmtime::Result<u32> {
        Ok(self.table.get(&this)?.texture.width())
    }
    fn height(&mut self, this: Resource<Texture>) -> wasmtime::Result<u32> {
        Ok(self.table.get(&this)?.texture.height())
    }
    fn depth_or_array_layers(&mut self, this: Resource<Texture>) -> wasmtime::Result<u32> {
        Ok(self.table.get(&this)?.texture.depth_or_array_layers())
    }
    fn format(&mut self, this: Resource<Texture>) -> wasmtime::Result<wit::TextureFormat> {
        let format = self.table.get(&this)?.texture.format();
        Ok(format_back(format).unwrap_or(wit::TextureFormat::Bgra8unorm))
    }
    fn drop(&mut self, rep: Resource<Texture>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

macro_rules! dropped {
    ($($trait:ident for $ty:ident),* $(,)?) => {$(
        impl wit::$trait for Guest {
            fn drop(&mut self, rep: Resource<$ty>) -> wasmtime::Result<()> {
                self.release(rep).map(drop)
            }
        }
    )*};
}
dropped!(
    HostTextureView for TextureView,
    HostSampler for Sampler,
    HostShaderModule for ShaderModule,
    HostBindGroupLayout for BindGroupLayout,
    HostPipelineLayout for PipelineLayout,
    HostBindGroup for BindGroup,
    HostCommandBuffer for CommandBuffer,
);

impl wit::HostRenderPipeline for Guest {
    fn get_bind_group_layout(
        &mut self,
        this: Resource<RenderPipeline>,
        index: u32,
    ) -> wasmtime::Result<Result<Resource<BindGroupLayout>, String>> {
        let pipeline = self.table.get(&this)?.0.clone();
        match self.checked(|_| Ok(pipeline.get_bind_group_layout(index))) {
            Ok(layout) => self.hold(BindGroupLayout(layout)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn drop(&mut self, rep: Resource<RenderPipeline>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}
impl wit::HostComputePipeline for Guest {
    fn get_bind_group_layout(
        &mut self,
        this: Resource<ComputePipeline>,
        index: u32,
    ) -> wasmtime::Result<Result<Resource<BindGroupLayout>, String>> {
        let pipeline = self.table.get(&this)?.0.clone();
        match self.checked(|_| Ok(pipeline.get_bind_group_layout(index))) {
            Ok(layout) => self.hold(BindGroupLayout(layout)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn drop(&mut self, rep: Resource<ComputePipeline>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostRenderPass for Guest {
    fn set_pipeline(
        &mut self,
        this: Resource<RenderPass>,
        pipeline: Resource<RenderPipeline>,
    ) -> wasmtime::Result<()> {
        let pipeline = self.table.get(&pipeline)?.0.clone();
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_pipeline(&pipeline);
        }
        Ok(())
    }
    fn set_bind_group(
        &mut self,
        this: Resource<RenderPass>,
        index: u32,
        group: Option<Resource<BindGroup>>,
        dynamic_offsets: Vec<u32>,
    ) -> wasmtime::Result<()> {
        let group = match &group {
            Some(group) => Some(self.table.get(group)?.0.clone()),
            None => None,
        };
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_bind_group(index, group.as_ref(), &dynamic_offsets);
        }
        Ok(())
    }
    fn set_vertex_buffer(
        &mut self,
        this: Resource<RenderPass>,
        slot: u32,
        buffer: Resource<Buffer>,
        offset: u64,
        size: Option<u64>,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&buffer)?;
        let range = match range(&buffer, offset, size) {
            Ok(range) => range,
            Err(why) => {
                self.misuse("set-vertex-buffer", why);
                return Ok(());
            }
        };
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_vertex_buffer(slot, buffer.slice(range));
        }
        Ok(())
    }
    fn set_index_buffer(
        &mut self,
        this: Resource<RenderPass>,
        buffer: Resource<Buffer>,
        index_format: wit::IndexFormat,
        offset: u64,
        size: Option<u64>,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&buffer)?;
        let range = match range(&buffer, offset, size) {
            Ok(range) => range,
            Err(why) => {
                self.misuse("set-index-buffer", why);
                return Ok(());
            }
        };
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_index_buffer(buffer.slice(range), index(index_format));
        }
        Ok(())
    }
    fn set_viewport(
        &mut self,
        this: Resource<RenderPass>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        min_depth: f32,
        max_depth: f32,
    ) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_viewport(x, y, width, height, min_depth, max_depth);
        }
        Ok(())
    }
    fn set_scissor_rect(
        &mut self,
        this: Resource<RenderPass>,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_scissor_rect(x, y, width, height);
        }
        Ok(())
    }
    fn set_blend_constant(
        &mut self,
        this: Resource<RenderPass>,
        c: wit::Color,
    ) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_blend_constant(color(c));
        }
        Ok(())
    }
    fn set_stencil_reference(
        &mut self,
        this: Resource<RenderPass>,
        reference: u32,
    ) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_stencil_reference(reference);
        }
        Ok(())
    }
    fn draw(
        &mut self,
        this: Resource<RenderPass>,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) -> wasmtime::Result<()> {
        let (Some(vertices), Some(instances)) = (
            first_vertex.checked_add(vertex_count),
            first_instance.checked_add(instance_count),
        ) else {
            self.misuse("draw", "counts past 2³²");
            return Ok(());
        };
        if !self.spend(1, 0, "draw") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.draw(first_vertex..vertices, first_instance..instances);
        }
        Ok(())
    }
    fn draw_indexed(
        &mut self,
        this: Resource<RenderPass>,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        base_vertex: i32,
        first_instance: u32,
    ) -> wasmtime::Result<()> {
        let (Some(indices), Some(instances)) = (
            first_index.checked_add(index_count),
            first_instance.checked_add(instance_count),
        ) else {
            self.misuse("draw-indexed", "counts past 2³²");
            return Ok(());
        };
        if !self.spend(1, 0, "draw-indexed") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.draw_indexed(first_index..indices, base_vertex, first_instance..instances);
        }
        Ok(())
    }
    fn draw_indirect(
        &mut self,
        this: Resource<RenderPass>,
        indirect: Resource<Buffer>,
        offset: u64,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&indirect)?;
        if !self.spend(1, 0, "draw-indirect") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.draw_indirect(&buffer, offset);
        }
        Ok(())
    }
    fn draw_indexed_indirect(
        &mut self,
        this: Resource<RenderPass>,
        indirect: Resource<Buffer>,
        offset: u64,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&indirect)?;
        if !self.spend(1, 0, "draw-indexed-indirect") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.draw_indexed_indirect(&buffer, offset);
        }
        Ok(())
    }
    fn execute_bundles(
        &mut self,
        this: Resource<RenderPass>,
        bundles: Vec<Resource<RenderBundle>>,
    ) -> wasmtime::Result<()> {
        let mut held = Vec::new();
        let mut draws = 0u64;
        for bundle in &bundles {
            let bundle = self.table.get(bundle)?;
            draws = draws.saturating_add(bundle.draws);
            held.push(bundle.bundle.clone());
        }
        if !self.spend(draws, 0, "execute-bundles") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.execute_bundles(held.iter());
        }
        Ok(())
    }
    fn begin_occlusion_query(
        &mut self,
        this: Resource<RenderPass>,
        query_index: u32,
    ) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.begin_occlusion_query(query_index);
        }
        Ok(())
    }
    fn end_occlusion_query(&mut self, this: Resource<RenderPass>) -> wasmtime::Result<()> {
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.end_occlusion_query();
        }
        Ok(())
    }
    fn end(&mut self, this: Resource<RenderPass>) -> wasmtime::Result<()> {
        self.table.get_mut(&this)?.0 = None;
        Ok(())
    }
    fn drop(&mut self, rep: Resource<RenderPass>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostComputePass for Guest {
    fn set_pipeline(
        &mut self,
        this: Resource<ComputePass>,
        pipeline: Resource<ComputePipeline>,
    ) -> wasmtime::Result<()> {
        let pipeline = self.table.get(&pipeline)?.0.clone();
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_pipeline(&pipeline);
        }
        Ok(())
    }
    fn set_bind_group(
        &mut self,
        this: Resource<ComputePass>,
        index: u32,
        group: Option<Resource<BindGroup>>,
        dynamic_offsets: Vec<u32>,
    ) -> wasmtime::Result<()> {
        let group = match &group {
            Some(group) => Some(self.table.get(group)?.0.clone()),
            None => None,
        };
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.set_bind_group(index, group.as_ref(), &dynamic_offsets);
        }
        Ok(())
    }
    fn dispatch_workgroups(
        &mut self,
        this: Resource<ComputePass>,
        x: u32,
        y: u32,
        z: u32,
    ) -> wasmtime::Result<()> {
        let groups = u64::from(x) * u64::from(y) * u64::from(z);
        if !self.spend(0, groups, "dispatch-workgroups") {
            return Ok(());
        }
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.dispatch_workgroups(x, y, z);
        }
        Ok(())
    }
    fn dispatch_workgroups_indirect(
        &mut self,
        this: Resource<ComputePass>,
        indirect: Resource<Buffer>,
        offset: u64,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&indirect)?;
        if let Some(pass) = &mut self.table.get_mut(&this)?.0 {
            pass.dispatch_workgroups_indirect(&buffer, offset);
        }
        Ok(())
    }
    fn end(&mut self, this: Resource<ComputePass>) -> wasmtime::Result<()> {
        self.table.get_mut(&this)?.0 = None;
        Ok(())
    }
    fn drop(&mut self, rep: Resource<ComputePass>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostCommandEncoder for Guest {
    fn begin_render_pass(
        &mut self,
        this: Resource<CommandEncoder>,
        descriptor: wit::RenderPassDescriptor,
    ) -> wasmtime::Result<Result<Resource<RenderPass>, String>> {
        let mut colors = Vec::new();
        for attachment in &descriptor.color_attachments {
            colors.push(match attachment {
                None => None,
                Some(a) => Some((
                    self.table.get(&a.view)?.0.clone(),
                    match &a.resolve_target {
                        Some(r) => Some(self.table.get(r)?.0.clone()),
                        None => None,
                    },
                    a.clear,
                    a.store,
                )),
            });
        }
        let depth = match &descriptor.depth_stencil_attachment {
            Some(d) => Some((self.table.get(&d.view)?.0.clone(), d)),
            None => None,
        };
        let timestamps = match &descriptor.timestamp_writes {
            Some(t) => Some((
                self.table.get(&t.query_set)?.set.clone(),
                t.beginning_index,
                t.end_index,
            )),
            None => None,
        };
        let occlusion = match &descriptor.occlusion_query_set {
            Some(set) => Some(self.table.get(set)?.set.clone()),
            None => None,
        };
        let Some(encoder) = &mut self.table.get_mut(&this)?.0 else {
            return Ok(Err("begin-render-pass: the encoder is finished".into()));
        };
        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = colors
            .iter()
            .map(|c| {
                c.as_ref().map(
                    |(view, resolve, clear, store)| wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: resolve.as_ref(),
                        ops: wgpu::Operations {
                            load: match clear {
                                Some(c) => wgpu::LoadOp::Clear(color(*c)),
                                None => wgpu::LoadOp::Load,
                            },
                            store: store_op(*store),
                        },
                    },
                )
            })
            .collect();
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: descriptor.label.as_deref(),
            color_attachments: &color_attachments,
            depth_stencil_attachment: depth.as_ref().map(|(view, d)| {
                wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: (!d.depth_read_only).then(|| wgpu::Operations {
                        load: match d.depth_clear {
                            Some(depth) => wgpu::LoadOp::Clear(depth),
                            None => wgpu::LoadOp::Load,
                        },
                        store: store_op(d.depth_store),
                    }),
                    // Neither said: the stencil is read only (a depth-only
                    // format's must be).
                    stencil_ops: (d.stencil_clear.is_some() || d.stencil_store.is_some()).then(
                        || wgpu::Operations {
                            load: match d.stencil_clear {
                                Some(value) => wgpu::LoadOp::Clear(value),
                                None => wgpu::LoadOp::Load,
                            },
                            store: d.stencil_store.map_or(wgpu::StoreOp::Store, store_op),
                        },
                    ),
                }
            }),
            timestamp_writes: timestamps.as_ref().map(|(set, beginning, end)| {
                wgpu::RenderPassTimestampWrites {
                    query_set: set,
                    beginning_of_pass_write_index: *beginning,
                    end_of_pass_write_index: *end,
                }
            }),
            occlusion_query_set: occlusion.as_ref(),
            ..Default::default()
        });
        let pass = pass.forget_lifetime();
        self.hold(RenderPass(Some(pass)))
    }
    fn begin_compute_pass(
        &mut self,
        this: Resource<CommandEncoder>,
        label: Option<String>,
        timestamp_writes: Option<wit::PassTimestampWrites>,
    ) -> wasmtime::Result<Result<Resource<ComputePass>, String>> {
        let timestamps = match &timestamp_writes {
            Some(t) => Some((
                self.table.get(&t.query_set)?.set.clone(),
                t.beginning_index,
                t.end_index,
            )),
            None => None,
        };
        let Some(encoder) = &mut self.table.get_mut(&this)?.0 else {
            return Ok(Err("begin-compute-pass: the encoder is finished".into()));
        };
        let pass = encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: label.as_deref(),
                timestamp_writes: timestamps.as_ref().map(|(set, beginning, end)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set: set,
                        beginning_of_pass_write_index: *beginning,
                        end_of_pass_write_index: *end,
                    }
                }),
            })
            .forget_lifetime();
        self.hold(ComputePass(Some(pass)))
    }
    fn resolve_query_set(
        &mut self,
        this: Resource<CommandEncoder>,
        query_set: Resource<QuerySet>,
        first_query: u32,
        query_count: u32,
        destination: Resource<Buffer>,
        destination_offset: u64,
    ) -> wasmtime::Result<()> {
        let set = self.table.get(&query_set)?.set.clone();
        let buffer = self.buffer(&destination)?;
        let Some(end) = first_query.checked_add(query_count) else {
            self.misuse("resolve-query-set", "queries past 2³²");
            return Ok(());
        };
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.resolve_query_set(&set, first_query..end, &buffer, destination_offset);
        }
        Ok(())
    }
    fn copy_buffer_to_buffer(
        &mut self,
        this: Resource<CommandEncoder>,
        source: Resource<Buffer>,
        source_offset: u64,
        destination: Resource<Buffer>,
        destination_offset: u64,
        size: u64,
    ) -> wasmtime::Result<()> {
        let (source, destination) = (self.buffer(&source)?, self.buffer(&destination)?);
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.copy_buffer_to_buffer(
                &source,
                source_offset,
                &destination,
                destination_offset,
                size,
            );
        }
        Ok(())
    }
    fn copy_buffer_to_texture(
        &mut self,
        this: Resource<CommandEncoder>,
        source: wit::BufferCopy,
        destination: wit::TextureCopy,
        size: wit::Extent,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&source.buffer)?;
        let (texture, mip_level, origin, aspect) = self.texture_copy(&destination)?;
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: layout(source.offset, source.bytes_per_row, source.rows_per_image),
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level,
                    origin,
                    aspect,
                },
                extent(size),
            );
        }
        Ok(())
    }
    fn copy_texture_to_buffer(
        &mut self,
        this: Resource<CommandEncoder>,
        source: wit::TextureCopy,
        destination: wit::BufferCopy,
        size: wit::Extent,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&destination.buffer)?;
        let (texture, mip_level, origin, aspect) = self.texture_copy(&source)?;
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level,
                    origin,
                    aspect,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: layout(
                        destination.offset,
                        destination.bytes_per_row,
                        destination.rows_per_image,
                    ),
                },
                extent(size),
            );
        }
        Ok(())
    }
    fn copy_texture_to_texture(
        &mut self,
        this: Resource<CommandEncoder>,
        source: wit::TextureCopy,
        destination: wit::TextureCopy,
        size: wit::Extent,
    ) -> wasmtime::Result<()> {
        let (from, from_mip, from_origin, from_aspect) = self.texture_copy(&source)?;
        let (to, to_mip, to_origin, to_aspect) = self.texture_copy(&destination)?;
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &from,
                    mip_level: from_mip,
                    origin: from_origin,
                    aspect: from_aspect,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &to,
                    mip_level: to_mip,
                    origin: to_origin,
                    aspect: to_aspect,
                },
                extent(size),
            );
        }
        Ok(())
    }
    fn clear_buffer(
        &mut self,
        this: Resource<CommandEncoder>,
        buffer: Resource<Buffer>,
        offset: u64,
        size: Option<u64>,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&buffer)?;
        if let Some(encoder) = &mut self.table.get_mut(&this)?.0 {
            encoder.clear_buffer(&buffer, offset, size);
        }
        Ok(())
    }
    fn finish(
        &mut self,
        this: Resource<CommandEncoder>,
    ) -> wasmtime::Result<Result<Resource<CommandBuffer>, String>> {
        let Some(encoder) = self.release(this)?.0 else {
            return Ok(Err("finish: the encoder is finished".into()));
        };
        match self.checked(|_| Ok(encoder.finish())) {
            Ok(buffer) => self.hold(CommandBuffer(buffer)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn drop(&mut self, rep: Resource<CommandEncoder>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::Host for Guest {
    fn ready(&mut self) -> wasmtime::Result<bool> {
        Ok(self.gpu.device.is_some())
    }
    fn get_adapter_info(&mut self) -> wasmtime::Result<Option<wit::AdapterInfo>> {
        Ok(self.gpu.device.as_ref().map(|d| {
            let info = d.device.adapter_info();
            wit::AdapterInfo {
                name: info.name.clone(),
                backend: match info.backend {
                    wgpu::Backend::Vulkan => "vulkan",
                    wgpu::Backend::Metal => "metal",
                    wgpu::Backend::Dx12 => "dx12",
                    wgpu::Backend::Gl => "gl",
                    _ => "other",
                }
                .into(),
            }
        }))
    }
    fn get_limits(&mut self) -> wasmtime::Result<wit::Limits> {
        let l = self
            .gpu
            .device
            .as_ref()
            .map(|d| d.device.limits())
            .unwrap_or_default();
        Ok(wit::Limits {
            max_texture_dimension_2d: l.max_texture_dimension_2d,
            max_texture_dimension_3d: l.max_texture_dimension_3d,
            max_buffer_size: l.max_buffer_size,
            max_bind_groups: l.max_bind_groups,
            max_vertex_buffers: l.max_vertex_buffers,
            max_uniform_buffer_binding_size: l.max_uniform_buffer_binding_size,
            max_storage_buffer_binding_size: l.max_storage_buffer_binding_size,
            max_compute_workgroup_size_x: l.max_compute_workgroup_size_x,
            max_compute_workgroup_size_y: l.max_compute_workgroup_size_y,
            max_compute_workgroup_size_z: l.max_compute_workgroup_size_z,
            max_compute_workgroups_per_dimension: l.max_compute_workgroups_per_dimension,
            min_uniform_buffer_offset_alignment: l.min_uniform_buffer_offset_alignment,
            min_storage_buffer_offset_alignment: l.min_storage_buffer_offset_alignment,
            max_session_bytes: self.gpu.max_bytes,
            max_session_objects: self.gpu.max_objects,
        })
    }
    fn preferred_format(&mut self) -> wasmtime::Result<wit::TextureFormat> {
        Ok(wit::TextureFormat::Bgra8unorm)
    }
    fn current_texture(&mut self) -> wasmtime::Result<Result<Resource<Texture>, String>> {
        match self.gpu.current_texture(&self.link) {
            Ok(texture) => self.hold(Texture { texture, bytes: 0 }),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_buffer(
        &mut self,
        d: wit::BufferDescriptor,
    ) -> wasmtime::Result<Result<Resource<Buffer>, String>> {
        let made = self.checked(|device| {
            Ok(device.device.create_buffer(&wgpu::BufferDescriptor {
                label: d.label.as_deref(),
                size: d.size,
                usage: buffer_usage(d.usage),
                mapped_at_creation: false,
            }))
        });
        match made {
            Ok(buffer) => self.hold(Buffer {
                buffer,
                usage: d.usage,
            }),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_buffer_with_data(
        &mut self,
        label: Option<String>,
        usage: wit::BufferUsage,
        contents: Vec<u8>,
    ) -> wasmtime::Result<Result<Resource<Buffer>, String>> {
        let made = self.checked(|device| {
            let size = (contents.len() as u64).div_ceil(4).max(1) * 4;
            let buffer = device.device.create_buffer(&wgpu::BufferDescriptor {
                label: label.as_deref(),
                size,
                usage: buffer_usage(usage) | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut padded = contents;
            padded.resize(size as usize, 0);
            device.queue.write_buffer(&buffer, 0, &padded);
            Ok(buffer)
        });
        match made {
            Ok(buffer) => self.hold(Buffer { buffer, usage }),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_texture(
        &mut self,
        d: wit::TextureDescriptor,
    ) -> wasmtime::Result<Result<Resource<Texture>, String>> {
        let fmt = format(d.format);
        let texel = u64::from(fmt.block_copy_size(None).unwrap_or(4));
        let mut bytes = u64::from(d.size.width)
            .saturating_mul(u64::from(d.size.height))
            .saturating_mul(u64::from(d.size.depth_or_array_layers))
            .saturating_mul(texel)
            .saturating_mul(u64::from(d.sample_count.max(1)));
        if d.mip_level_count > 1 {
            bytes = bytes.saturating_mul(4) / 3;
        }
        if self.gpu.bytes.saturating_add(bytes) > self.gpu.max_bytes {
            return Ok(Err(format!(
                "a texture of {bytes} bytes would pass the session's {} of GPU memory",
                self.gpu.max_bytes
            )));
        }
        let view_formats: Vec<wgpu::TextureFormat> =
            d.view_formats.iter().map(|f| format(*f)).collect();
        let made = self.checked(|device| {
            Ok(device.device.create_texture(&wgpu::TextureDescriptor {
                label: d.label.as_deref(),
                size: extent(d.size),
                mip_level_count: d.mip_level_count,
                sample_count: d.sample_count,
                dimension: match d.dimension {
                    wit::TextureDimension::D1 => wgpu::TextureDimension::D1,
                    wit::TextureDimension::D2 => wgpu::TextureDimension::D2,
                    wit::TextureDimension::D3 => wgpu::TextureDimension::D3,
                },
                format: fmt,
                usage: texture_usage(d.usage),
                view_formats: &view_formats,
            }))
        });
        match made {
            Ok(texture) => self.hold(Texture { texture, bytes }),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_sampler(
        &mut self,
        d: wit::SamplerDescriptor,
    ) -> wasmtime::Result<Result<Resource<Sampler>, String>> {
        let made = self.checked(|device| {
            Ok(device.device.create_sampler(&wgpu::SamplerDescriptor {
                label: d.label.as_deref(),
                address_mode_u: address(d.address_mode_u),
                address_mode_v: address(d.address_mode_v),
                address_mode_w: address(d.address_mode_w),
                mag_filter: filter(d.mag_filter),
                min_filter: filter(d.min_filter),
                mipmap_filter: match d.mipmap_filter {
                    wit::FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
                    wit::FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
                },
                lod_min_clamp: d.lod_min_clamp,
                lod_max_clamp: d.lod_max_clamp,
                compare: d.compare.map(compare),
                anisotropy_clamp: d.max_anisotropy.max(1),
                border_color: None,
            }))
        });
        match made {
            Ok(sampler) => self.hold(Sampler(sampler)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_shader_module(
        &mut self,
        label: Option<String>,
        wgsl: String,
    ) -> wasmtime::Result<Result<Resource<ShaderModule>, String>> {
        if wgsl.len() > MAX_WGSL {
            return Ok(Err(format!(
                "a shader of {} bytes: the most is {MAX_WGSL}",
                wgsl.len()
            )));
        }
        // Its loops counted against the budget of an invocation.
        let module = match crate::loops::bounded(&wgsl, self.gpu.loop_budget) {
            Ok((module, note)) => {
                if let Some(note) = note {
                    self.log.line(format!("gpu: {note}"));
                }
                module
            }
            Err(why) => return Ok(Err(why)),
        };
        let made = self.checked(|device| {
            Ok(device
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: label.as_deref(),
                    source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
                }))
        });
        match made {
            Ok(module) => self.hold(ShaderModule(module)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_bind_group_layout(
        &mut self,
        label: Option<String>,
        entries: Vec<wit::BindGroupLayoutEntry>,
    ) -> wasmtime::Result<Result<Resource<BindGroupLayout>, String>> {
        let entries: Vec<wgpu::BindGroupLayoutEntry> = entries
            .iter()
            .map(|e| wgpu::BindGroupLayoutEntry {
                binding: e.binding,
                visibility: stages(e.visibility),
                ty: binding_type(&e.binding_type),
                count: None,
            })
            .collect();
        let made = self.checked(|device| {
            Ok(device
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: label.as_deref(),
                    entries: &entries,
                }))
        });
        match made {
            Ok(layout) => self.hold(BindGroupLayout(layout)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_pipeline_layout(
        &mut self,
        label: Option<String>,
        bind_group_layouts: Vec<Resource<BindGroupLayout>>,
    ) -> wasmtime::Result<Result<Resource<PipelineLayout>, String>> {
        let mut layouts = Vec::new();
        for layout in &bind_group_layouts {
            layouts.push(self.table.get(layout)?.0.clone());
        }
        let refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
        let made = self.checked(|device| {
            Ok(device
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: label.as_deref(),
                    bind_group_layouts: &refs,
                    immediate_size: 0,
                }))
        });
        match made {
            Ok(layout) => self.hold(PipelineLayout(layout)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_bind_group(
        &mut self,
        label: Option<String>,
        layout: Resource<BindGroupLayout>,
        entries: Vec<wit::BindGroupEntry>,
    ) -> wasmtime::Result<Result<Resource<BindGroup>, String>> {
        enum Bound {
            Buffer(wgpu::Buffer, u64, Option<NonZeroU64>),
            Sampler(wgpu::Sampler),
            View(wgpu::TextureView),
        }
        let layout = self.table.get(&layout)?.0.clone();
        let mut bound = Vec::new();
        for entry in &entries {
            bound.push((
                entry.binding,
                match &entry.resource {
                    wit::BindingResource::Buffer(b) => Bound::Buffer(
                        self.buffer(&b.buffer)?,
                        b.offset,
                        b.size.and_then(NonZeroU64::new),
                    ),
                    wit::BindingResource::Sampler(s) => {
                        Bound::Sampler(self.table.get(s)?.0.clone())
                    }
                    wit::BindingResource::TextureView(v) => {
                        Bound::View(self.table.get(v)?.0.clone())
                    }
                },
            ));
        }
        let entries: Vec<wgpu::BindGroupEntry<'_>> = bound
            .iter()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match b {
                    Bound::Buffer(buffer, offset, size) => {
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: *offset,
                            size: *size,
                        })
                    }
                    Bound::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                    Bound::View(view) => wgpu::BindingResource::TextureView(view),
                },
            })
            .collect();
        let made = self.checked(|device| {
            Ok(device.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: label.as_deref(),
                layout: &layout,
                entries: &entries,
            }))
        });
        match made {
            Ok(group) => self.hold(BindGroup(group)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_render_pipeline(
        &mut self,
        d: wit::RenderPipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<RenderPipeline>, String>> {
        let parts = self.render_parts(&d)?;
        match self.checked(|device| Ok(parts.create(&device.device))) {
            Ok(pipeline) => self.hold(RenderPipeline(pipeline)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_compute_pipeline(
        &mut self,
        d: wit::ComputePipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<ComputePipeline>, String>> {
        let parts = self.compute_parts(&d)?;
        match self.checked(|device| Ok(parts.create(&device.device))) {
            Ok(pipeline) => self.hold(ComputePipeline(pipeline)),
            Err(why) => Ok(Err(why)),
        }
    }
    fn create_render_pipeline_async(
        &mut self,
        d: wit::RenderPipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<crate::pipelines::RenderPipelineFuture>, String>> {
        self.render_pipeline_async(d)
    }
    fn create_compute_pipeline_async(
        &mut self,
        d: wit::ComputePipelineDescriptor,
    ) -> wasmtime::Result<Result<Resource<crate::pipelines::ComputePipelineFuture>, String>> {
        self.compute_pipeline_async(d)
    }
    fn create_render_bundle_encoder(
        &mut self,
        d: wit::RenderBundleEncoderDescriptor,
    ) -> wasmtime::Result<Result<Resource<crate::bundles::RenderBundleEncoder>, String>> {
        self.bundle_encoder(d)
    }
    fn create_query_set(
        &mut self,
        d: wit::QuerySetDescriptor,
    ) -> wasmtime::Result<Result<Resource<QuerySet>, String>> {
        self.query_set(d)
    }
    fn supports_timestamps(&mut self) -> wasmtime::Result<bool> {
        Ok(self.gpu.device.as_ref().is_some_and(|d| d.timestamps))
    }
    fn get_timestamp_period(&mut self) -> wasmtime::Result<f32> {
        Ok(self
            .gpu
            .device
            .as_ref()
            .map_or(1., |d| d.queue.get_timestamp_period()))
    }
    fn create_command_encoder(
        &mut self,
        label: Option<String>,
    ) -> wasmtime::Result<Result<Resource<CommandEncoder>, String>> {
        let made = self.checked(|device| {
            Ok(device
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: label.as_deref(),
                }))
        });
        match made {
            Ok(encoder) => self.hold(CommandEncoder(Some(encoder))),
            Err(why) => Ok(Err(why)),
        }
    }
    fn write_buffer(
        &mut self,
        buffer: Resource<Buffer>,
        offset: u64,
        data: Vec<u8>,
    ) -> wasmtime::Result<Result<(), String>> {
        let buffer = self.buffer(&buffer)?;
        Ok(self.checked(|device| {
            if data.is_empty() {
                return Ok(());
            }
            range(&buffer, offset, Some(data.len() as u64))
                .map_err(|why| format!("write-buffer: {why}"))?;
            device.queue.write_buffer(&buffer, offset, &data);
            Ok(())
        }))
    }
    fn write_texture(
        &mut self,
        destination: wit::TextureCopy,
        data: Vec<u8>,
        bytes_per_row: Option<u32>,
        rows_per_image: Option<u32>,
        size: wit::Extent,
    ) -> wasmtime::Result<Result<(), String>> {
        let (texture, mip_level, origin, aspect) = self.texture_copy(&destination)?;
        Ok(self.checked(|device| {
            device.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level,
                    origin,
                    aspect,
                },
                &data,
                layout(0, bytes_per_row, rows_per_image),
                extent(size),
            );
            Ok(())
        }))
    }
    fn submit(
        &mut self,
        buffers: Vec<Resource<CommandBuffer>>,
    ) -> wasmtime::Result<Result<(), String>> {
        let mut commands = Vec::new();
        for buffer in buffers {
            commands.push(self.release(buffer)?.0);
        }
        // What was recorded since the last submission, counted: past the
        // caps, none of it runs.
        self.gpu.draws = 0;
        self.gpu.workgroups = 0;
        if let Some(over) = self.gpu.over.take() {
            return Ok(Err(format!("submit: refused, {over}")));
        }
        // One submission on the GPU at a time: a GPU turns to another
        // device's work between submissions (not inside one, on NVIDIA's
        // driver), so zgx's frames get it between the guest's. Past the
        // watchdog, the session ends.
        self.gpu.settle().map_err(wasmtime::Error::msg)?;
        Ok(self.checked(|device| {
            device.queue.submit(commands);
            Ok(())
        }))
    }
    fn read_buffer(
        &mut self,
        buffer: Resource<Buffer>,
        offset: u64,
        size: u64,
    ) -> wasmtime::Result<Result<Vec<u8>, String>> {
        let held = self.table.get(&buffer)?;
        if !held.usage.contains(wit::BufferUsage::MAP_READ) {
            return Ok(Err("read-buffer: the buffer has no `map-read` usage".into()));
        }
        if size > MAX_READ {
            return Ok(Err(format!(
                "read-buffer: {size} bytes; the most is {MAX_READ}"
            )));
        }
        let buffer = held.buffer.clone();
        Ok(self.checked(|device| {
            let range =
                range(&buffer, offset, Some(size)).map_err(|why| format!("read-buffer: {why}"))?;
            let slice = buffer.slice(range);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |mapped| {
                let _ = tx.send(mapped);
            });
            let _ = device.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(2)),
            });
            match rx.try_recv() {
                Ok(Ok(())) => {
                    let bytes = slice.get_mapped_range().to_vec();
                    buffer.unmap();
                    Ok(bytes)
                }
                Ok(Err(error)) => Err(format!("read-buffer: {error}")),
                Err(_) => Err("read-buffer: the GPU took longer than two seconds".into()),
            }
        }))
    }
}

fn layout(
    offset: u64,
    bytes_per_row: Option<u32>,
    rows_per_image: Option<u32>,
) -> wgpu::TexelCopyBufferLayout {
    wgpu::TexelCopyBufferLayout {
        offset,
        bytes_per_row,
        rows_per_image,
    }
}
fn extent(e: wit::Extent) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: e.width,
        height: e.height,
        depth_or_array_layers: e.depth_or_array_layers,
    }
}
fn color(c: wit::Color) -> wgpu::Color {
    wgpu::Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}
fn store_op(store: wit::StoreOp) -> wgpu::StoreOp {
    match store {
        wit::StoreOp::Store => wgpu::StoreOp::Store,
        wit::StoreOp::Discard => wgpu::StoreOp::Discard,
    }
}
fn aspect(a: wit::TextureAspect) -> wgpu::TextureAspect {
    match a {
        wit::TextureAspect::All => wgpu::TextureAspect::All,
        wit::TextureAspect::StencilOnly => wgpu::TextureAspect::StencilOnly,
        wit::TextureAspect::DepthOnly => wgpu::TextureAspect::DepthOnly,
    }
}
pub(crate) fn index(f: wit::IndexFormat) -> wgpu::IndexFormat {
    match f {
        wit::IndexFormat::Uint16 => wgpu::IndexFormat::Uint16,
        wit::IndexFormat::Uint32 => wgpu::IndexFormat::Uint32,
    }
}
fn address(a: wit::AddressMode) -> wgpu::AddressMode {
    match a {
        wit::AddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        wit::AddressMode::Repeat => wgpu::AddressMode::Repeat,
        wit::AddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}
fn filter(f: wit::FilterMode) -> wgpu::FilterMode {
    match f {
        wit::FilterMode::Nearest => wgpu::FilterMode::Nearest,
        wit::FilterMode::Linear => wgpu::FilterMode::Linear,
    }
}
pub(crate) fn compare(c: wit::CompareFunction) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match c {
        wit::CompareFunction::Never => C::Never,
        wit::CompareFunction::Less => C::Less,
        wit::CompareFunction::Equal => C::Equal,
        wit::CompareFunction::LessEqual => C::LessEqual,
        wit::CompareFunction::Greater => C::Greater,
        wit::CompareFunction::NotEqual => C::NotEqual,
        wit::CompareFunction::GreaterEqual => C::GreaterEqual,
        wit::CompareFunction::Always => C::Always,
    }
}
pub(crate) fn topology(t: wit::PrimitiveTopology) -> wgpu::PrimitiveTopology {
    use wgpu::PrimitiveTopology as P;
    match t {
        wit::PrimitiveTopology::PointList => P::PointList,
        wit::PrimitiveTopology::LineList => P::LineList,
        wit::PrimitiveTopology::LineStrip => P::LineStrip,
        wit::PrimitiveTopology::TriangleList => P::TriangleList,
        wit::PrimitiveTopology::TriangleStrip => P::TriangleStrip,
    }
}
pub(crate) fn blend(b: wit::BlendComponent) -> wgpu::BlendComponent {
    use wgpu::BlendFactor as F;
    let factor = |f: wit::BlendFactor| match f {
        wit::BlendFactor::Zero => F::Zero,
        wit::BlendFactor::One => F::One,
        wit::BlendFactor::Src => F::Src,
        wit::BlendFactor::OneMinusSrc => F::OneMinusSrc,
        wit::BlendFactor::SrcAlpha => F::SrcAlpha,
        wit::BlendFactor::OneMinusSrcAlpha => F::OneMinusSrcAlpha,
        wit::BlendFactor::Dst => F::Dst,
        wit::BlendFactor::OneMinusDst => F::OneMinusDst,
        wit::BlendFactor::DstAlpha => F::DstAlpha,
        wit::BlendFactor::OneMinusDstAlpha => F::OneMinusDstAlpha,
        wit::BlendFactor::SrcAlphaSaturated => F::SrcAlphaSaturated,
        wit::BlendFactor::Constant => F::Constant,
        wit::BlendFactor::OneMinusConstant => F::OneMinusConstant,
    };
    wgpu::BlendComponent {
        src_factor: factor(b.src_factor),
        dst_factor: factor(b.dst_factor),
        operation: match b.operation {
            wit::BlendOperation::Add => wgpu::BlendOperation::Add,
            wit::BlendOperation::Subtract => wgpu::BlendOperation::Subtract,
            wit::BlendOperation::ReverseSubtract => wgpu::BlendOperation::ReverseSubtract,
            wit::BlendOperation::Min => wgpu::BlendOperation::Min,
            wit::BlendOperation::Max => wgpu::BlendOperation::Max,
        },
    }
}
pub(crate) fn writes(w: wit::ColorWrite) -> wgpu::ColorWrites {
    let mut out = wgpu::ColorWrites::empty();
    for (ours, theirs) in [
        (wit::ColorWrite::RED, wgpu::ColorWrites::RED),
        (wit::ColorWrite::GREEN, wgpu::ColorWrites::GREEN),
        (wit::ColorWrite::BLUE, wgpu::ColorWrites::BLUE),
        (wit::ColorWrite::ALPHA, wgpu::ColorWrites::ALPHA),
    ] {
        if w.contains(ours) {
            out |= theirs;
        }
    }
    out
}
fn stages(s: wit::ShaderStage) -> wgpu::ShaderStages {
    let mut out = wgpu::ShaderStages::empty();
    for (ours, theirs) in [
        (wit::ShaderStage::VERTEX, wgpu::ShaderStages::VERTEX),
        (wit::ShaderStage::FRAGMENT, wgpu::ShaderStages::FRAGMENT),
        (wit::ShaderStage::COMPUTE, wgpu::ShaderStages::COMPUTE),
    ] {
        if s.contains(ours) {
            out |= theirs;
        }
    }
    out
}
fn buffer_usage(u: wit::BufferUsage) -> wgpu::BufferUsages {
    use wgpu::BufferUsages as B;
    let mut out = B::empty();
    for (ours, theirs) in [
        (wit::BufferUsage::MAP_READ, B::MAP_READ),
        (wit::BufferUsage::MAP_WRITE, B::MAP_WRITE),
        (wit::BufferUsage::COPY_SRC, B::COPY_SRC),
        (wit::BufferUsage::COPY_DST, B::COPY_DST),
        (wit::BufferUsage::INDEX, B::INDEX),
        (wit::BufferUsage::VERTEX, B::VERTEX),
        (wit::BufferUsage::UNIFORM, B::UNIFORM),
        (wit::BufferUsage::STORAGE, B::STORAGE),
        (wit::BufferUsage::INDIRECT, B::INDIRECT),
        (wit::BufferUsage::QUERY_RESOLVE, B::QUERY_RESOLVE),
    ] {
        if u.contains(ours) {
            out |= theirs;
        }
    }
    out
}
fn texture_usage(u: wit::TextureUsage) -> wgpu::TextureUsages {
    use wgpu::TextureUsages as T;
    let mut out = T::empty();
    for (ours, theirs) in [
        (wit::TextureUsage::COPY_SRC, T::COPY_SRC),
        (wit::TextureUsage::COPY_DST, T::COPY_DST),
        (wit::TextureUsage::TEXTURE_BINDING, T::TEXTURE_BINDING),
        (wit::TextureUsage::STORAGE_BINDING, T::STORAGE_BINDING),
        (wit::TextureUsage::RENDER_ATTACHMENT, T::RENDER_ATTACHMENT),
    ] {
        if u.contains(ours) {
            out |= theirs;
        }
    }
    out
}
fn view_dimension(d: wit::TextureViewDimension) -> wgpu::TextureViewDimension {
    use wgpu::TextureViewDimension as V;
    match d {
        wit::TextureViewDimension::D1 => V::D1,
        wit::TextureViewDimension::D2 => V::D2,
        wit::TextureViewDimension::D2Array => V::D2Array,
        wit::TextureViewDimension::Cube => V::Cube,
        wit::TextureViewDimension::CubeArray => V::CubeArray,
        wit::TextureViewDimension::D3 => V::D3,
    }
}
fn binding_type(t: &wit::BindingType) -> wgpu::BindingType {
    match t {
        wit::BindingType::Buffer(b) => wgpu::BindingType::Buffer {
            ty: match b.kind {
                wit::BufferBindingType::Uniform => wgpu::BufferBindingType::Uniform,
                wit::BufferBindingType::Storage => {
                    wgpu::BufferBindingType::Storage { read_only: false }
                }
                wit::BufferBindingType::ReadOnlyStorage => {
                    wgpu::BufferBindingType::Storage { read_only: true }
                }
            },
            has_dynamic_offset: b.has_dynamic_offset,
            min_binding_size: b.min_binding_size.and_then(NonZeroU64::new),
        },
        wit::BindingType::Sampler(s) => wgpu::BindingType::Sampler(match s {
            wit::SamplerBindingType::Filtering => wgpu::SamplerBindingType::Filtering,
            wit::SamplerBindingType::NonFiltering => wgpu::SamplerBindingType::NonFiltering,
            wit::SamplerBindingType::Comparison => wgpu::SamplerBindingType::Comparison,
        }),
        wit::BindingType::Texture(t) => wgpu::BindingType::Texture {
            sample_type: match t.sample_type {
                wit::TextureSampleType::Float => {
                    wgpu::TextureSampleType::Float { filterable: true }
                }
                wit::TextureSampleType::UnfilterableFloat => {
                    wgpu::TextureSampleType::Float { filterable: false }
                }
                wit::TextureSampleType::Depth => wgpu::TextureSampleType::Depth,
                wit::TextureSampleType::Sint => wgpu::TextureSampleType::Sint,
                wit::TextureSampleType::Uint => wgpu::TextureSampleType::Uint,
            },
            view_dimension: view_dimension(t.view_dimension),
            multisampled: t.multisampled,
        },
        wit::BindingType::StorageTexture(t) => wgpu::BindingType::StorageTexture {
            access: match t.access {
                wit::StorageTextureAccess::WriteOnly => wgpu::StorageTextureAccess::WriteOnly,
                wit::StorageTextureAccess::ReadOnly => wgpu::StorageTextureAccess::ReadOnly,
                wit::StorageTextureAccess::ReadWrite => wgpu::StorageTextureAccess::ReadWrite,
            },
            format: format(t.format),
            view_dimension: view_dimension(t.view_dimension),
        },
    }
}
pub(crate) fn vertex_format(f: wit::VertexFormat) -> wgpu::VertexFormat {
    use wgpu::VertexFormat as V;
    use wit::VertexFormat as W;
    match f {
        W::Uint8x2 => V::Uint8x2,
        W::Uint8x4 => V::Uint8x4,
        W::Sint8x2 => V::Sint8x2,
        W::Sint8x4 => V::Sint8x4,
        W::Unorm8x2 => V::Unorm8x2,
        W::Unorm8x4 => V::Unorm8x4,
        W::Snorm8x2 => V::Snorm8x2,
        W::Snorm8x4 => V::Snorm8x4,
        W::Uint16x2 => V::Uint16x2,
        W::Uint16x4 => V::Uint16x4,
        W::Sint16x2 => V::Sint16x2,
        W::Sint16x4 => V::Sint16x4,
        W::Unorm16x2 => V::Unorm16x2,
        W::Unorm16x4 => V::Unorm16x4,
        W::Snorm16x2 => V::Snorm16x2,
        W::Snorm16x4 => V::Snorm16x4,
        W::Float16x2 => V::Float16x2,
        W::Float16x4 => V::Float16x4,
        W::Float32 => V::Float32,
        W::Float32x2 => V::Float32x2,
        W::Float32x3 => V::Float32x3,
        W::Float32x4 => V::Float32x4,
        W::Uint32 => V::Uint32,
        W::Uint32x2 => V::Uint32x2,
        W::Uint32x3 => V::Uint32x3,
        W::Uint32x4 => V::Uint32x4,
        W::Sint32 => V::Sint32,
        W::Sint32x2 => V::Sint32x2,
        W::Sint32x3 => V::Sint32x3,
        W::Sint32x4 => V::Sint32x4,
    }
}
const FORMATS: [(wit::TextureFormat, wgpu::TextureFormat); 23] = {
    use wgpu::TextureFormat as T;
    use wit::TextureFormat as W;
    [
        (W::R8unorm, T::R8Unorm),
        (W::R8snorm, T::R8Snorm),
        (W::R8uint, T::R8Uint),
        (W::R8sint, T::R8Sint),
        (W::Rg8unorm, T::Rg8Unorm),
        (W::Rgba8unorm, T::Rgba8Unorm),
        (W::Rgba8unormSrgb, T::Rgba8UnormSrgb),
        (W::Bgra8unorm, T::Bgra8Unorm),
        (W::Bgra8unormSrgb, T::Bgra8UnormSrgb),
        (W::R16float, T::R16Float),
        (W::Rg16float, T::Rg16Float),
        (W::Rgba16float, T::Rgba16Float),
        (W::R32float, T::R32Float),
        (W::Rg32float, T::Rg32Float),
        (W::Rgba32float, T::Rgba32Float),
        (W::R32uint, T::R32Uint),
        (W::R32sint, T::R32Sint),
        (W::Rgba32uint, T::Rgba32Uint),
        (W::Rgb10a2unorm, T::Rgb10a2Unorm),
        (W::Depth16unorm, T::Depth16Unorm),
        (W::Depth24plus, T::Depth24Plus),
        (W::Depth24plusStencil8, T::Depth24PlusStencil8),
        (W::Depth32float, T::Depth32Float),
    ]
};
pub(crate) fn format(f: wit::TextureFormat) -> wgpu::TextureFormat {
    FORMATS
        .iter()
        .find(|(ours, _)| *ours == f)
        .map(|(_, theirs)| *theirs)
        .unwrap_or(wgpu::TextureFormat::Bgra8Unorm)
}
fn format_back(f: wgpu::TextureFormat) -> Option<wit::TextureFormat> {
    FORMATS
        .iter()
        .find(|(_, theirs)| *theirs == f)
        .map(|(ours, _)| *ours)
}
