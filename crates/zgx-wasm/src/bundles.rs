//! Render bundles (draws recorded once, run by many passes) and query sets
//! (occlusion, and timestamps where the adapter has them).
use crate::{
    gpu::{BindGroup, Buffer, NO_DEVICE, RenderPipeline, format, index, range},
    runtime::{Guest, bindings::zgx::app::gpu as wit},
};
use std::ops::Range;
use wasmtime::component::Resource;
use zgx_gpu::wgpu;

/// Queries in a set, at most (WebGPU's).
const MAX_QUERIES: u32 = 4096;

pub struct QuerySet {
    pub(crate) set: wgpu::QuerySet,
    kind: wit::QueryType,
    count: u32,
}
pub struct RenderBundle {
    pub(crate) bundle: wgpu::RenderBundle,
    /// Its draws, counted against a submission's as a pass executes it.
    pub(crate) draws: u64,
}
/// A bundle being recorded: its commands kept with the objects they use,
/// replayed into wgpu's encoder as it finishes, so no borrow outlives a
/// host call.
pub struct RenderBundleEncoder {
    label: Option<String>,
    color_formats: Vec<Option<wgpu::TextureFormat>>,
    depth_stencil: Option<wgpu::RenderBundleDepthStencil>,
    sample_count: u32,
    commands: Vec<Command>,
    draws: u64,
}
enum Command {
    Pipeline(wgpu::RenderPipeline),
    BindGroup(u32, Option<wgpu::BindGroup>, Vec<u32>),
    Vertex(u32, wgpu::Buffer, Range<u64>),
    Index(wgpu::Buffer, wgpu::IndexFormat, Range<u64>),
    Draw(Range<u32>, Range<u32>),
    DrawIndexed(Range<u32>, i32, Range<u32>),
    DrawIndirect(wgpu::Buffer, u64),
    DrawIndexedIndirect(wgpu::Buffer, u64),
}
impl crate::gpu::Charged for QuerySet {}
impl crate::gpu::Charged for RenderBundle {}
impl crate::gpu::Charged for RenderBundleEncoder {}

impl Guest {
    pub(crate) fn query_set(
        &mut self,
        d: wit::QuerySetDescriptor,
    ) -> wasmtime::Result<Result<Resource<QuerySet>, String>> {
        let timestamps = self.gpu.device.as_ref().is_some_and(|d| d.timestamps);
        if d.kind == wit::QueryType::Timestamp && !timestamps {
            return Ok(Err(
                "timestamp queries: this GPU has none (`supports-timestamps` is false)".into(),
            ));
        }
        if d.count == 0 || d.count > MAX_QUERIES {
            return Ok(Err(format!(
                "a query set of {}: 1 to {MAX_QUERIES}",
                d.count
            )));
        }
        let made = self.checked(|device| {
            Ok(device.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: d.label.as_deref(),
                ty: match d.kind {
                    wit::QueryType::Occlusion => wgpu::QueryType::Occlusion,
                    wit::QueryType::Timestamp => wgpu::QueryType::Timestamp,
                },
                count: d.count,
            }))
        });
        match made {
            Ok(set) => self.hold(QuerySet {
                set,
                kind: d.kind,
                count: d.count,
            }),
            Err(why) => Ok(Err(why)),
        }
    }
    pub(crate) fn bundle_encoder(
        &mut self,
        d: wit::RenderBundleEncoderDescriptor,
    ) -> wasmtime::Result<Result<Resource<RenderBundleEncoder>, String>> {
        if self.gpu.device.is_none() {
            return Ok(Err(NO_DEVICE.into()));
        }
        self.hold(RenderBundleEncoder {
            label: d.label,
            color_formats: d.color_formats.iter().map(|f| f.map(format)).collect(),
            depth_stencil: d.depth_stencil.map(|ds| wgpu::RenderBundleDepthStencil {
                format: format(ds.format),
                depth_read_only: ds.depth_read_only,
                stencil_read_only: ds.stencil_read_only,
            }),
            sample_count: d.sample_count,
            commands: Vec::new(),
            draws: 0,
        })
    }
    fn record(
        &mut self,
        this: &Resource<RenderBundleEncoder>,
        command: Command,
    ) -> wasmtime::Result<()> {
        let encoder = self.table.get_mut(this)?;
        if matches!(
            command,
            Command::Draw(..)
                | Command::DrawIndexed(..)
                | Command::DrawIndirect(..)
                | Command::DrawIndexedIndirect(..)
        ) {
            encoder.draws += 1;
        }
        encoder.commands.push(command);
        Ok(())
    }
}

impl wit::HostQuerySet for Guest {
    fn count(&mut self, this: Resource<QuerySet>) -> wasmtime::Result<u32> {
        Ok(self.table.get(&this)?.count)
    }
    fn kind(&mut self, this: Resource<QuerySet>) -> wasmtime::Result<wit::QueryType> {
        Ok(self.table.get(&this)?.kind)
    }
    fn drop(&mut self, rep: Resource<QuerySet>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostRenderBundle for Guest {
    fn drop(&mut self, rep: Resource<RenderBundle>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}

impl wit::HostRenderBundleEncoder for Guest {
    fn set_pipeline(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        pipeline: Resource<RenderPipeline>,
    ) -> wasmtime::Result<()> {
        let pipeline = self.table.get(&pipeline)?.0.clone();
        self.record(&this, Command::Pipeline(pipeline))
    }
    fn set_bind_group(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        index: u32,
        group: Option<Resource<BindGroup>>,
        dynamic_offsets: Vec<u32>,
    ) -> wasmtime::Result<()> {
        let group = match &group {
            Some(group) => Some(self.table.get(group)?.0.clone()),
            None => None,
        };
        self.record(&this, Command::BindGroup(index, group, dynamic_offsets))
    }
    fn set_vertex_buffer(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        slot: u32,
        buffer: Resource<Buffer>,
        offset: u64,
        size: Option<u64>,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&buffer)?;
        match range(&buffer, offset, size) {
            Ok(range) => self.record(&this, Command::Vertex(slot, buffer, range)),
            Err(why) => {
                self.misuse("set-vertex-buffer", why);
                Ok(())
            }
        }
    }
    fn set_index_buffer(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        buffer: Resource<Buffer>,
        index_format: wit::IndexFormat,
        offset: u64,
        size: Option<u64>,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&buffer)?;
        match range(&buffer, offset, size) {
            Ok(range) => self.record(&this, Command::Index(buffer, index(index_format), range)),
            Err(why) => {
                self.misuse("set-index-buffer", why);
                Ok(())
            }
        }
    }
    fn draw(
        &mut self,
        this: Resource<RenderBundleEncoder>,
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
        self.record(
            &this,
            Command::Draw(first_vertex..vertices, first_instance..instances),
        )
    }
    fn draw_indexed(
        &mut self,
        this: Resource<RenderBundleEncoder>,
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
        self.record(
            &this,
            Command::DrawIndexed(first_index..indices, base_vertex, first_instance..instances),
        )
    }
    fn draw_indirect(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        indirect: Resource<Buffer>,
        offset: u64,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&indirect)?;
        self.record(&this, Command::DrawIndirect(buffer, offset))
    }
    fn draw_indexed_indirect(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        indirect: Resource<Buffer>,
        offset: u64,
    ) -> wasmtime::Result<()> {
        let buffer = self.buffer(&indirect)?;
        self.record(&this, Command::DrawIndexedIndirect(buffer, offset))
    }
    fn finish(
        &mut self,
        this: Resource<RenderBundleEncoder>,
        label: Option<String>,
    ) -> wasmtime::Result<Result<Resource<RenderBundle>, String>> {
        let recorded = self.release(this)?;
        let draws = recorded.draws;
        let made = self.checked(|device| {
            let mut encoder =
                device
                    .device
                    .create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
                        label: recorded.label.as_deref(),
                        color_formats: &recorded.color_formats,
                        depth_stencil: recorded.depth_stencil,
                        sample_count: recorded.sample_count,
                        multiview: None,
                    });
            for command in &recorded.commands {
                match command {
                    Command::Pipeline(pipeline) => encoder.set_pipeline(pipeline),
                    Command::BindGroup(index, group, offsets) => {
                        encoder.set_bind_group(*index, group.as_ref(), offsets)
                    }
                    Command::Vertex(slot, buffer, range) => {
                        encoder.set_vertex_buffer(*slot, buffer.slice(range.clone()))
                    }
                    Command::Index(buffer, format, range) => {
                        encoder.set_index_buffer(buffer.slice(range.clone()), *format)
                    }
                    Command::Draw(vertices, instances) => {
                        encoder.draw(vertices.clone(), instances.clone())
                    }
                    Command::DrawIndexed(indices, base, instances) => {
                        encoder.draw_indexed(indices.clone(), *base, instances.clone())
                    }
                    Command::DrawIndirect(buffer, offset) => encoder.draw_indirect(buffer, *offset),
                    Command::DrawIndexedIndirect(buffer, offset) => {
                        encoder.draw_indexed_indirect(buffer, *offset)
                    }
                }
            }
            Ok(encoder.finish(&wgpu::RenderBundleDescriptor {
                label: label.as_deref(),
            }))
        });
        match made {
            Ok(bundle) => self.hold(RenderBundle { bundle, draws }),
            Err(why) => Ok(Err(why)),
        }
    }
    fn drop(&mut self, rep: Resource<RenderBundleEncoder>) -> wasmtime::Result<()> {
        self.release(rep).map(drop)
    }
}
