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
/// Commands a bundle may record, at most.
const MAX_BUNDLE_COMMANDS: usize = 65536;

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
    /// What its commands hold of the host's memory, charged to the
    /// session's GPU bytes as they are recorded.
    bytes: u64,
    /// Why it can't be finished: past a cap.
    refused: Option<String>,
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
impl crate::gpu::Charged for RenderBundleEncoder {
    fn bytes(&self) -> u64 {
        self.bytes
    }
}

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
        let Some(device) = &self.gpu.device else {
            return Ok(Err(NO_DEVICE.into()));
        };
        let color_formats: Vec<Option<wgpu::TextureFormat>> =
            d.color_formats.iter().map(|f| f.map(format)).collect();
        let depth_stencil = d.depth_stencil.map(|ds| wgpu::RenderBundleDepthStencil {
            format: format(ds.format),
            depth_read_only: ds.depth_read_only,
            stencil_read_only: ds.stencil_read_only,
        });
        // Checked here: wgpu panics on some descriptors it doesn't take.
        let features = device.device.features();
        let renderable = |f: wgpu::TextureFormat| {
            f.guaranteed_format_features(features)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        };
        let max_colors = device.device.limits().max_color_attachments as usize;
        let why = if !matches!(d.sample_count, 1 | 4) {
            Some(format!("a sample count of {}: 1 or 4", d.sample_count))
        } else if color_formats.len() > max_colors {
            Some(format!(
                "{} colour formats: the device takes {max_colors}",
                color_formats.len()
            ))
        } else if color_formats
            .iter()
            .flatten()
            .any(|f| f.is_depth_stencil_format() || !renderable(*f))
        {
            Some("a colour format no colour attachment has".into())
        } else if depth_stencil
            .as_ref()
            .is_some_and(|ds| !ds.format.is_depth_stencil_format())
        {
            Some("a depth-stencil format that isn't one".into())
        } else if color_formats.iter().all(Option::is_none) && depth_stencil.is_none() {
            Some("no attachment at all".into())
        } else {
            None
        };
        if let Some(why) = why {
            return Ok(Err(format!("create-render-bundle-encoder: {why}")));
        }
        self.hold(RenderBundleEncoder {
            label: d.label,
            color_formats,
            depth_stencil,
            sample_count: d.sample_count,
            commands: Vec::new(),
            draws: 0,
            bytes: 0,
            refused: None,
        })
    }
    /// `command` kept for the bundle, within its caps: as many commands,
    /// a submission's draws, and the session's bytes.
    fn record(
        &mut self,
        this: &Resource<RenderBundleEncoder>,
        command: Command,
    ) -> wasmtime::Result<()> {
        let bytes = (std::mem::size_of::<Command>()
            + match &command {
                Command::BindGroup(_, _, offsets) => offsets.len() * 4,
                _ => 0,
            }) as u64;
        let draw = u64::from(matches!(
            command,
            Command::Draw(..)
                | Command::DrawIndexed(..)
                | Command::DrawIndirect(..)
                | Command::DrawIndexedIndirect(..)
        ));
        let (max_draws, max_bytes, held) =
            (self.gpu.caps.draws, self.gpu.max_bytes, self.gpu.bytes);
        let encoder = self.table.get_mut(this)?;
        if encoder.refused.is_some() {
            return Ok(());
        }
        let why = if encoder.commands.len() >= MAX_BUNDLE_COMMANDS {
            Some(format!("past a bundle's {MAX_BUNDLE_COMMANDS} commands"))
        } else if encoder.draws + draw > max_draws {
            Some(format!("past a submission's {max_draws} draws"))
        } else if held.saturating_add(bytes) > max_bytes {
            Some(format!("past the session's {max_bytes} bytes"))
        } else {
            None
        };
        if let Some(why) = why {
            encoder.refused = Some(why.clone());
            encoder.commands = Vec::new();
            let freed = std::mem::take(&mut encoder.bytes);
            self.gpu.bytes = self.gpu.bytes.saturating_sub(freed);
            self.misuse("render-bundle-encoder", why);
            return Ok(());
        }
        encoder.draws += draw;
        encoder.bytes += bytes;
        encoder.commands.push(command);
        self.gpu.bytes += bytes;
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
        if let Some(why) = &recorded.refused {
            return Ok(Err(format!("finish: refused, {why}")));
        }
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
