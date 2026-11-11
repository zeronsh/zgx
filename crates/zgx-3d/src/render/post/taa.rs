//! Temporal antialiasing on the device (`Antialiasing::Taa`): the frame's
//! jitter, two history textures traded each frame, and the pass blending
//! what the meshes drew (`Resource::Fresh`) with the last frame's history
//! into the scene and the next history at once.
use super::super::graph::{Graph, SCENE};
use crate::math::Mat4;
use std::time::Instant;
use zgx_gpu::wgpu;

/// Frames jittered in turn, and the frames after a change a world draws
/// before it settles (design-3d-aaa.md, 4.4).
pub(in crate::render) const FRAMES: u32 = 8;

/// The least share of each frame in the blend (the rest the history's):
/// a young history's frames share equally (1 / its frames), so a still
/// view's first `FRAMES` average as many samples, then this.
const SHARE: f32 = 0.1;

/// The least share of a frame in which some mesh changed (moved, posed,
/// added): until the prepass follows each mesh's motion, such a frame
/// leans on itself, so a moving mesh trails two frames at most.
const MOVING: f32 = 0.5;

/// A frame's share of the blend, after `frames` in the history: equal to
/// theirs while it's young, then `SHARE`; at least `MOVING` where something
/// `moved` (a mesh, or a material by the time).
fn share(frames: u32, moved: bool) -> f32 {
    let share = (1. / (frames + 1) as f32).max(SHARE);
    if moved { share.max(MOVING) } else { share }
}

/// The pass's uniforms (the shader's `Taa`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Uniforms {
    back: [[f32; 4]; 4],
    before: [[f32; 4]; 4],
    params: [f32; 4],
}

/// The frame `index`'s offset from the pixel's centre, in pixels: Halton's
/// (2, 3) sequence, `FRAMES` long.
pub(in crate::render) fn jitter(index: u64) -> (f32, f32) {
    let halton = |mut i: u32, base: u32| {
        let (mut f, mut r) = (1f32, 0f32);
        while i > 0 {
            f /= base as f32;
            r += f * (i % base) as f32;
            i /= base;
        }
        r
    };
    let i = (index % u64::from(FRAMES)) as u32 + 1;
    (halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// `projection` moved by `jitter` pixels of a frame of `size`.
pub(in crate::render) fn jittered(projection: Mat4, (x, y): (f32, f32), size: (u32, u32)) -> Mat4 {
    let mut shift = Mat4::IDENTITY;
    shift.columns[3][0] = 2. * x / size.0.max(1) as f32;
    shift.columns[3][1] = -2. * y / size.1.max(1) as f32;
    shift * projection
}

struct History {
    size: (u32, u32),
    views: [wgpu::TextureView; 2],
    /// The depth each frame drew at, beside it (what moved is told by it).
    depths: [wgpu::TextureView; 2],
    _textures: [wgpu::Texture; 4],
}

/// The kept depth's format.
const DEPTH_KEPT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// What temporal antialiasing keeps of the frames before, for the frames of
/// one size (one view, or each box of a view drawn in two): its history,
/// and where the blend is.
struct Track {
    /// The size it is for.
    size: (u32, u32),
    /// The renderer's tick it last ran at.
    ran: Option<u32>,
    uniforms: wgpu::Buffer,
    history: Option<History>,
    /// The bindings reading each history, for the compile they were made for.
    binds: [Option<wgpu::BindGroup>; 2],
    compiled: u64,
    step: Step,
    /// The renderer's tick (`render::Tick::index`) it last ran at, and how
    /// it was before: a view drawn again in the same frame runs as it did,
    /// adding no frame.
    began: Option<(u32, Step)>,
}

/// Where a track's blend is: which history holds the last frame, whether
/// it holds one to blend and of how many frames; last frame's view and
/// projection (unjittered), time and the world's cuts then; the world's
/// meshes and changes as of the last frame, and frames drawn since.
#[derive(Clone, Copy)]
struct Step {
    /// Frames drawn at its size: its history is made once a size has stood
    /// a frame (`STEADY`), not each frame of a resize drag.
    stood: u32,
    at: usize,
    valid: bool,
    frames: u32,
    before: Option<Mat4>,
    last: Option<Instant>,
    cuts: u64,
    meshes: u64,
    revision: u64,
    since: u32,
}

/// What a frame's blend follows: this frame drawn with `view_projection`
/// (unjittered) at `time`, the world cut `cuts` times so far, its meshes
/// and changes as counted (`Inner::meshes`, `revision`), the renderer's
/// tick.
pub(in crate::render) struct At {
    pub(in crate::render) view_projection: Mat4,
    pub(in crate::render) time: Instant,
    pub(in crate::render) cuts: u64,
    pub(in crate::render) meshes: u64,
    pub(in crate::render) revision: u64,
    pub(in crate::render) tick: u32,
    /// Something on screen moves by itself (a material animated by
    /// `frame.time`): blended as a frame in which a mesh moved.
    pub(in crate::render) animated: bool,
}

/// The most sizes kept at once (boxes of one view drawn at sizes of their
/// own, in one frame): a box past them is drawn without a history.
const TRACKS: usize = 4;

/// Frames a size has stood before its history is made: a resize drag
/// draws without one (and allocates nothing) until it stops.
const STEADY: u32 = 2;

pub(super) struct Taa {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tracks: Vec<Track>,
}

impl Taa {
    #[cold]
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D TAA"),
            source: wgpu::ShaderSource::Wgsl(include_str!("taa.wgsl").into()),
        });
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D TAA"),
            entries: &[
                texture(0, float),
                texture(1, wgpu::TextureSampleType::Depth),
                texture(2, float),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(5, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D TAA"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D TAA"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("blend"),
                compilation_options: Default::default(),
                targets: &[
                    Some(SCENE.into()),
                    Some(SCENE.into()),
                    Some(DEPTH_KEPT.into()),
                ],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("3D TAA"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            tracks: Vec::new(),
        }
    }

    /// Lets go of its histories (the next frame starts one).
    pub(super) fn rest(&mut self) {
        self.tracks.clear();
    }

    /// The bytes its histories hold: two frames' colour (8 bytes a pixel)
    /// and depth (4) each.
    pub(super) fn bytes(&self) -> u64 {
        let track = |t: &Track| {
            t.history.as_ref().map_or(0, |h| {
                2 * u64::from(h.size.0) * u64::from(h.size.1) * (8 + 4)
            })
        };
        self.tracks.iter().map(track).sum()
    }

    /// The blend into the scene and the next history of the track of the
    /// frame's size, `at` (see `At`): whether the tracks drawn this tick
    /// have frames still to gather, and whether it blended (else the scene
    /// is to be what the meshes drew: a size that hasn't stood yet).
    pub(super) fn run(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        at: &At,
        look: (f32, f32),
    ) -> (bool, bool) {
        let Some(size) = graph.targets.as_ref().map(|t| t.size) else {
            return (false, false);
        };
        // Tracks not drawn for a second go (a box gone).
        self.tracks.retain(|t| {
            t.size == size
                || t.step
                    .last
                    .is_some_and(|last| at.time.saturating_duration_since(last).as_secs_f32() < 1.)
        });
        let found = self.tracks.iter().position(|t| t.size == size);
        let others = self.tracks.iter().any(|t| t.ran == Some(at.tick));
        let index = match found {
            Some(index) => index,
            // The view was resized (no box drew at this tick yet): its
            // track made again at the new size.
            None if !others && !self.tracks.is_empty() => {
                let last = (0..self.tracks.len())
                    .max_by_key(|&i| self.tracks[i].ran)
                    .unwrap_or(0);
                self.tracks[last] = Track::new(device, size, false);
                last
            }
            // Another box at a size of its own: a track of its own, up to
            // `TRACKS` (none of this tick's given up for it).
            None if self.tracks.len() < TRACKS => {
                self.tracks.push(Track::new(device, size, true));
                self.tracks.len() - 1
            }
            // Or in place of one no box drew at this tick (a box gone).
            None if let Some(idle) = (0..self.tracks.len())
                .filter(|&i| self.tracks[i].ran != Some(at.tick))
                .min_by_key(|&i| self.tracks[i].ran) =>
            {
                self.tracks[idle] = Track::new(device, size, true);
                idle
            }
            // Past them: drawn as it is, no history.
            None => {
                let gathering = (self.tracks.iter())
                    .filter(|t| t.ran == Some(at.tick))
                    .any(Track::gathering);
                return (gathering, false);
            }
        };
        let (pipeline, layout, sampler) = (&self.pipeline, &self.layout, &self.sampler);
        let track = &mut self.tracks[index];
        let blended = track.run(
            (device, queue),
            encoder,
            graph,
            at,
            look,
            (pipeline, layout, sampler),
        );
        track.ran = Some(at.tick);
        let gathering = (self.tracks.iter())
            .filter(|t| t.ran == Some(at.tick))
            .any(Track::gathering);
        (gathering, blended)
    }
}

impl Track {
    /// A track for frames of `size`; `ready` to blend from its first (a
    /// view's first frames, a second box), else once the size has stood (a
    /// resize).
    fn new(device: &wgpu::Device, size: (u32, u32), ready: bool) -> Self {
        Self {
            size,
            ran: None,
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D TAA"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            history: None,
            binds: [None, None],
            compiled: 0,
            step: Step {
                stood: if ready { STEADY } else { 0 },
                at: 0,
                valid: false,
                frames: 0,
                before: None,
                last: None,
                cuts: 0,
                meshes: u64::MAX,
                revision: u64::MAX,
                since: 0,
            },
            began: None,
        }
    }

    fn run(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        encoder: &mut wgpu::CommandEncoder,
        graph: &Graph,
        at: &At,
        (exposure, tone): (f32, f32),
        (pipeline, layout, sampler): (
            &wgpu::RenderPipeline,
            &wgpu::BindGroupLayout,
            &wgpu::Sampler,
        ),
    ) -> bool {
        // Drawn again at the same tick: as it ran then.
        match self.began {
            Some((tick, before)) if tick == at.tick => self.step = before,
            _ => self.began = Some((at.tick, self.step)),
        }
        let step = &mut self.step;
        if step.revision == at.revision {
            step.since += 1;
        } else {
            step.revision = at.revision;
            step.since = 0;
        }
        step.stood = step.stood.saturating_add(1);
        let (view_projection, time, cuts, meshes) =
            (at.view_projection, at.time, at.cuts, at.meshes);
        let Some(targets) = graph.targets.as_ref() else {
            return false;
        };
        let (Some(scene), Some(fresh)) = (targets.scene.as_ref(), targets.others[3].as_ref())
        else {
            return false;
        };
        let size = targets.size;
        // A size not yet stood (a resize drag): the frame as drawn, nothing
        // made for it.
        if self.step.stood < STEADY {
            self.step.last = Some(time);
            return false;
        }
        if self.history.as_ref().is_none_or(|h| h.size != size) {
            let make = |format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("3D TAA history"),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let textures = [make(SCENE), make(SCENE), make(DEPTH_KEPT), make(DEPTH_KEPT)];
            let views = [0, 1].map(|i| textures[i].create_view(&Default::default()));
            let depths = [2, 3].map(|i| textures[i].create_view(&Default::default()));
            self.history = Some(History {
                size,
                views,
                depths,
                _textures: textures,
            });
            self.binds = [None, None];
            self.step.valid = false;
        }
        if self.compiled != graph.compiled {
            self.binds = [None, None];
            self.compiled = graph.compiled;
        }
        // A cut, or a gap past a quarter of a second: no history.
        let gap = self
            .step
            .last
            .is_none_or(|last| time.saturating_duration_since(last).as_secs_f32() > 0.25);
        if gap || cuts != self.step.cuts {
            self.step.valid = false;
        }
        self.step.last = Some(time);
        self.step.cuts = cuts;
        let Some(history) = &self.history else {
            return false;
        };
        let back = view_projection.inverse().unwrap_or(Mat4::IDENTITY);
        let before = self.step.before.unwrap_or(view_projection);
        if !self.step.valid {
            self.step.frames = 0;
        }
        let share = share(self.step.frames, meshes != self.step.meshes || at.animated);
        self.step.meshes = meshes;
        let uniforms = Uniforms {
            back: back.columns,
            before: before.columns,
            params: [share, f32::from(u8::from(self.step.valid)), exposure, tone],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let (read, write) = (self.step.at, 1 - self.step.at);
        let bind = self.binds[read].get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("3D TAA"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(fresh),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&history.views[read]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&history.depths[read]),
                    },
                ],
            })
        });
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D TAA"),
            color_attachments: &[
                attachment(scene),
                attachment(&history.views[write]),
                attachment(&history.depths[write]),
            ],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*bind, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        self.step.at = write;
        self.step.valid = true;
        self.step.frames += 1;
        self.step.before = Some(view_projection);
        true
    }

    /// Whether it has frames still to gather: since the world last changed,
    /// or into a history young (just made, cut, after a gap), or none yet.
    fn gathering(&self) -> bool {
        self.history.is_none() || self.step.since + 1 < FRAMES || self.step.frames < FRAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_in_which_something_moved_weighs_at_least_half() {
        assert_eq!(share(0, false), 1.);
        assert_eq!(share(20, false), SHARE);
        assert_eq!(share(20, true), MOVING);
    }

    #[test]
    fn jitter_covers_the_pixel_in_eight_frames_and_repeats() {
        let offsets: Vec<(f32, f32)> = (0..8).map(jitter).collect();
        for &(x, y) in &offsets {
            assert!((-0.5..0.5).contains(&x) && (-0.5..0.5).contains(&y));
        }
        // Each offset its own; the ninth the first again.
        for (i, a) in offsets.iter().enumerate() {
            assert!(offsets[..i].iter().all(|b| b != a));
        }
        assert_eq!(jitter(8), offsets[0]);
        // Their mean near the centre.
        let mean = offsets
            .iter()
            .fold((0., 0.), |m, o| (m.0 + o.0 / 8., m.1 + o.1 / 8.));
        assert!(mean.0.abs() < 0.1 && mean.1.abs() < 0.1, "{mean:?}");
    }
}
