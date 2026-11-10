//! Runs of meshes in a pass: what one draw holds, and binding only what
//! differs from the run before.
use super::{Gpu, Variant};
use crate::texture::MAPS;
use zgx_gpu::wgpu;
/// Meshes drawn together: of one geometry, maps and variant.
pub(super) struct Run {
    pub(super) geometry: u64,
    pub(super) glass: bool,
    pub(super) maps: [u64; MAPS],
    pub(super) variant: Variant,
    pub(super) instances: std::ops::Range<u32>,
}

/// What a pass has bound, by address, so a run sets only what differs from
/// the run before (setting the same again changes nothing but costs wgpu a
/// recorded and checked command).
#[derive(Default)]
pub(super) struct Bound {
    pub(super) pipeline: Option<*const wgpu::RenderPipeline>,
    pub(super) groups: [Option<*const wgpu::BindGroup>; 4],
    pub(super) vertices: [Option<*const wgpu::Buffer>; 6],
    pub(super) indices: Option<*const wgpu::Buffer>,
}
impl Bound {
    pub(super) fn pipeline(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
    ) {
        if self.pipeline.replace(pipeline) != Some(pipeline as *const _) {
            pass.set_pipeline(pipeline);
        }
    }
    pub(super) fn group(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        index: u32,
        group: &wgpu::BindGroup,
    ) {
        if self.groups[index as usize].replace(group) != Some(group as *const _) {
            pass.set_bind_group(index, group, &[]);
        }
    }
    pub(super) fn vertices(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        slot: u32,
        buffer: &wgpu::Buffer,
    ) {
        if self.vertices[slot as usize].replace(buffer) != Some(buffer as *const _) {
            pass.set_vertex_buffer(slot, buffer.slice(..));
        }
    }
    pub(super) fn indices(&mut self, pass: &mut wgpu::RenderPass<'_>, buffer: &wgpu::Buffer) {
        if self.indices.replace(buffer) != Some(buffer as *const _) {
            pass.set_index_buffer(buffer.slice(..), wgpu::IndexFormat::Uint32);
        }
    }
}

/// Draws `run` in `pass`, its frame and instances bound, setting only
/// what differs from what `bound` says the pass has. (Inline: called for
/// every run, it was otherwise a call a run.)
#[inline(always)]
pub(super) fn draw_run(pass: &mut wgpu::RenderPass<'_>, gpu: &Gpu, run: &Run, bound: &mut Bound) {
    let buffers = &gpu.geometries[&run.geometry];
    let maps = gpu.bindings.get(&run.maps).map(|(binding, _)| binding);
    let skin = buffers.skin.as_ref().filter(|_| run.variant.skinned());
    let morphs = buffers.morphs.as_ref().filter(|_| run.variant.morphing());
    let palette = gpu
        .palettes
        .as_ref()
        .filter(|_| run.variant.skinned() || run.variant.morphing());
    match gpu.variants.get(&run.variant) {
        Some(pipelines) => {
            bound.pipeline(pass, &pipelines[usize::from(run.glass)]);
            bound.group(pass, 1, maps.unwrap_or(&gpu.empty_bind));
            if let Some((_, _, palette)) = palette {
                bound.group(pass, 2, palette);
            }
            if let Some(skin) = skin {
                bound.vertices(pass, 2, skin);
            }
            if let Some(uvs) = buffers.second_uvs.as_ref().filter(|_| run.variant.placed()) {
                let slot = 2 + u32::from(run.variant.skinned());
                bound.vertices(pass, slot, uvs);
            }
            if let Some(colors) = buffers.colors.as_ref().filter(|_| run.variant.colored()) {
                let (skinned, placed) = (run.variant.skinned(), run.variant.placed());
                let slot = 2 + u32::from(skinned) + u32::from(placed);
                bound.vertices(pass, slot, colors);
            }
            if let Some(morphs) = morphs {
                bound.group(pass, 3, morphs);
            }
        }
        None => bound.pipeline(pass, &gpu.plain[usize::from(run.glass)]),
    }
    bound.vertices(pass, 0, &buffers.vertices);
    bound.indices(pass, &buffers.indices);
    pass.draw_indexed(0..buffers.count, 0, run.instances.clone());
}
/// The meshes' pass: each run drawn into the frame's targets, resolved
/// into `target` (multisampled) or straight into it, cleared to `clear`.
pub(super) fn draw_main(
    encoder: &mut wgpu::CommandEncoder,
    gpu: &Gpu,
    runs: &[Run],
    target: &wgpu::TextureView,
    frame_bind: &wgpu::BindGroup,
    clear: wgpu::Color,
) {
    let Some(targets) = &gpu.graph.targets else {
        return;
    };
    let (colour, depth_view) = (&targets.colour, &targets.depth);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("3D"),
        // Multisampled, resolved into the target; else into it.
        color_attachments: &[Some(match colour {
            Some(colour) => wgpu::RenderPassColorAttachment {
                view: colour,
                depth_slice: None,
                resolve_target: Some(target),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Discard,
                },
            },
            None => wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    let Some((instances_buffer, _)) = &gpu.instances else {
        return;
    };
    pass.set_bind_group(0, frame_bind, &[]);
    pass.set_vertex_buffer(1, instances_buffer.slice(..));
    // Each run sets only what differs from the run before: wgpu records
    // and checks every call, the same pipeline or buffer again too.
    let mut bound = Bound::default();
    for run in runs.iter() {
        draw_run(&mut pass, gpu, run, &mut bound);
    }
    drop(pass);
}
