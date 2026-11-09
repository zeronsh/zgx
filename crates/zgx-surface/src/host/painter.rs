//! The GPU view's painter: it answers the client's hello with what the
//! renderer's device can share, makes a generation of buffers at the box's
//! size, and draws the newest presented buffer over the whole box.
use super::{AppInner, RESIZE_DEBOUNCE, RING};
use crate::{
    iosurface::HostChannel,
    protocol::{
        BufferDesc, BufferKind, Generation, HostMessage, Offer, PROTOCOL_VERSION, PixelFormat,
        Welcome,
    },
    ring::Slot,
    shm::Mapping,
};
use std::{rc::Weak, sync::atomic::Ordering, time::Instant};
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

pub(crate) struct Painter {
    app: Weak<AppInner>,
    gpu: Option<Blit>,
    generation: Option<Buffers>,
    /// What shows: a generation's buffer (Vulkan) or upload (shm).
    shown: Option<Shown>,
    /// When the box first had a size its generation isn't.
    resizing: Option<Instant>,
    /// The scale the client was last told.
    told_scale: f32,
    log: Log,
}

/// `ZGX_SURFACE_LOG=1`: every 240 frames shown, how long they waited from
/// the client's present to the host drawing them (then a refresh or so to
/// the screen), on stderr.
struct Log {
    on: bool,
    waits: Vec<f64>,
}
impl Log {
    fn presented(&mut self, at: Option<Instant>, pending: bool) {
        let (true, true, Some(at)) = (self.on, pending, at) else {
            return;
        };
        self.waits.push(at.elapsed().as_secs_f64() * 1000.);
        if self.waits.len() == 240 {
            self.waits.sort_by(f64::total_cmp);
            let mean = self.waits.iter().sum::<f64>() / self.waits.len() as f64;
            eprintln!(
                "SURFACE present-to-drawn ms: mean {mean:.2} median {:.2} p95 {:.2} max {:.2}",
                self.waits[120], self.waits[228], self.waits[239]
            );
            self.waits.clear();
        }
    }
}
struct Shown {
    bind: wgpu::BindGroup,
    // Kept alive while it shows, past its generation.
    _texture: wgpu::Texture,
}
/// A generation's buffers, on the host's side.
struct Buffers {
    generation: u32,
    size: (u32, u32),
    kind: BufferKind,
    /// Vulkan: the shared textures, by index.
    textures: Vec<wgpu::Texture>,
    /// shm: each buffer's mapping, and the texture they are uploaded into.
    maps: Vec<Mapping>,
    upload: Option<wgpu::Texture>,
}
struct Blit {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Painter {
    pub(crate) fn new(app: Weak<AppInner>) -> Self {
        Self {
            app,
            gpu: None,
            generation: None,
            shown: None,
            resizing: None,
            told_scale: 0.,
            log: Log {
                on: std::env::var_os("ZGX_SURFACE_LOG").is_some_and(|v| v != "0"),
                waits: Vec::new(),
            },
        }
    }
}

impl Viewport for Painter {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let Some(app) = self.app.upgrade().filter(|app| !app.revoked.get()) else {
            // Gone or revoked: nothing of its frames is kept.
            self.shown = None;
            self.generation = None;
            clear(frame, None);
            return;
        };
        app.scale.set(frame.scale);
        app.tell_refresh();
        if app.device.borrow().as_ref() != Some(frame.device) {
            *app.device.borrow_mut() = Some(frame.device.clone());
        }
        // The client said hello: how this device shares, and on which GPU.
        if app.kind.get().is_none()
            && let Some(hello) = app.hello.borrow_mut().take()
        {
            let (kind, device) = choose(&hello.offers, frame.device, app.mach.as_deref());
            app.kind.set(Some(kind));
            app.out.send(HostMessage::Welcome(Welcome {
                version: PROTOCOL_VERSION,
                kind,
                device,
            }));
        }
        if let Some(kind) = app.kind.get() {
            self.size_generation(&app, kind, frame);
        }
        if self
            .gpu
            .as_ref()
            .is_none_or(|blit| blit.format != frame.format)
        {
            self.gpu = Some(Blit::new(frame.device, frame.format));
        }
        self.show_newest(&app, frame);
        clear(frame, self.shown.as_ref().zip(self.gpu.as_ref()));
    }
    fn device_changed(&mut self) {
        // Its buffers were the old device's: a new generation on the next
        // frame, at the same size.
        self.gpu = None;
        self.shown = None;
        self.generation = None;
        self.resizing = None;
        if let Some(app) = self.app.upgrade() {
            // Welcomed for the old device: what the new one shares may
            // differ, but the client can't take a new welcome. Keep its kind.
            *app.device.borrow_mut() = None;
        }
    }
}

impl Painter {
    /// A generation at the box's size: at once for the first, else once a
    /// new size has held for `RESIZE_DEBOUNCE` (stretched meanwhile).
    fn size_generation(
        &mut self,
        app: &std::rc::Rc<AppInner>,
        kind: BufferKind,
        frame: &ViewportFrame<'_>,
    ) {
        let size = (frame.size.0.max(1), frame.size.1.max(1));
        let current = self.generation.as_ref().map(|g| g.size);
        if current == Some(size) {
            self.resizing = None;
            if self.told_scale != frame.scale {
                self.told_scale = frame.scale;
                app.out.send(HostMessage::Resize {
                    width: size.0,
                    height: size.1,
                    scale: frame.scale,
                });
            }
            return;
        }
        if current.is_some() {
            let since = *self.resizing.get_or_insert(frame.time);
            let held = frame.time.saturating_duration_since(since);
            if held < RESIZE_DEBOUNCE {
                app.invalidate_after(RESIZE_DEBOUNCE - held);
                return;
            }
        }
        self.resizing = None;
        let gone = match kind {
            BufferKind::Vulkan => !crate::wgpu29::can_share(frame.device),
            BufferKind::IoSurface => !crate::wgpu29::can_share_iosurface(frame.device),
            _ => false,
        };
        let kind = if gone {
            // Welcomed as Vulkan or IOSurfaces on a device that has gone (a
            // new renderer on another backend): nothing it can share.
            return;
        } else {
            kind
        };
        let generation = app.shared.lock().unwrap().ring.next_generation(RING);
        let made = match kind {
            BufferKind::Vulkan => vulkan_generation(frame, generation, size),
            BufferKind::IoSurface => match app.mach.as_deref() {
                Some(mach) => iosurface_generation(frame, generation, size, mach),
                None => return,
            },
            _ => shm_generation(frame, generation, size),
        };
        let (buffers, message) = match made {
            Ok(made) => made,
            Err(error) => {
                eprintln!(
                    "zgx-surface: buffers of {}x{} couldn't be made: {error}",
                    size.0, size.1
                );
                return;
            }
        };
        self.generation = Some(buffers);
        self.told_scale = frame.scale;
        app.out.send(HostMessage::Resize {
            width: size.0,
            height: size.1,
            scale: frame.scale,
        });
        app.out.send(HostMessage::Buffers(message));
    }

    /// The newest frame presented, made the one shown; the one it replaces
    /// released once the GPU has read it.
    fn show_newest(&mut self, app: &std::rc::Rc<AppInner>, frame: &mut ViewportFrame<'_>) {
        let (Some(buffers), Some(blit)) = (&self.generation, &self.gpu) else {
            return;
        };
        match buffers.kind {
            BufferKind::Vulkan | BufferKind::IoSurface => {
                let newest = {
                    let mut shared = app.shared.lock().unwrap();
                    self.log
                        .presented(shared.presented_at.take(), shared.ring.has_pending());
                    shared.ring.show_newest()
                };
                let Some((slot, previous)) = newest else {
                    return;
                };
                if slot.generation != buffers.generation {
                    return;
                }
                let texture = buffers.textures[slot.index as usize].clone();
                self.shown = Some(Shown {
                    bind: blit.bind(frame.device, &texture),
                    _texture: texture,
                });
                if let Some(previous) = previous {
                    // The frames that read it were submitted before this
                    // one: called back once they're done.
                    let (out, awaiting) = (app.out.clone(), app.awaiting.clone());
                    awaiting.fetch_add(1, Ordering::AcqRel);
                    frame.queue.on_submitted_work_done(move || {
                        out.send(release(previous));
                        awaiting.fetch_sub(1, Ordering::AcqRel);
                    });
                    app.poll_until_released();
                }
            }
            _ => {
                let newest = {
                    let mut shared = app.shared.lock().unwrap();
                    self.log
                        .presented(shared.presented_at.take(), shared.ring.has_pending());
                    shared.ring.show_newest()
                };
                let Some((slot, previous)) = newest else {
                    return;
                };
                // The one it replaces was copied as it showed: the client's
                // again now. Held until then, it paces the client to the
                // frames shown, as a Vulkan buffer's release does.
                if let Some(previous) = previous {
                    app.out.send(release(previous));
                }
                let (Some(upload), Some(map)) =
                    (&buffers.upload, buffers.maps.get(slot.index as usize))
                else {
                    return;
                };
                if slot.generation != buffers.generation {
                    return;
                }
                let (width, height) = buffers.size;
                // Safety: presented, the buffer is the host's until released.
                let pixels = unsafe { map.bytes() };
                frame.queue.write_texture(
                    upload.as_image_copy(),
                    &pixels[..(width * height * 4) as usize],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                if self.shown.as_ref().is_none_or(|s| s._texture != *upload) {
                    self.shown = Some(Shown {
                        bind: blit.bind(frame.device, upload),
                        _texture: upload.clone(),
                    });
                }
            }
        }
    }
}

fn release(slot: Slot) -> HostMessage {
    HostMessage::Release {
        generation: slot.generation,
        index: slot.index,
    }
}

/// How to share with a client offering `offers` (best first), from this
/// device: Vulkan images where it is on one of the client's GPUs and can
/// export them, IOSurfaces where it is on one of the client's Metal devices
/// and the client checked in on the Mach channel (`mach`), else shared
/// memory.
fn choose(
    offers: &[Offer],
    device: &wgpu::Device,
    mach: Option<&HostChannel>,
) -> (BufferKind, Option<crate::DeviceIds>) {
    let forced_shm = std::env::var_os("ZGX_SURFACE_SHM").is_some_and(|v| v != "0");
    let ours = (!forced_shm && crate::wgpu29::can_share(device))
        .then(|| crate::wgpu29::device_ids_of(device))
        .flatten();
    let metal = (!forced_shm && mach.is_some_and(HostChannel::is_connected))
        .then(|| crate::wgpu29::iosurface_ids_of(device))
        .flatten();
    for offer in offers {
        match offer {
            Offer::Vulkan { devices } if ours.is_some_and(|ids| devices.contains(&ids)) => {
                return (BufferKind::Vulkan, ours);
            }
            Offer::IoSurface { devices }
                if metal
                    .and_then(|ids| ids.metal_registry_id())
                    .is_some_and(|id| devices.contains(&id)) =>
            {
                return (BufferKind::IoSurface, metal);
            }
            Offer::Shm => return (BufferKind::Shm, None),
            _ => {}
        }
    }
    (BufferKind::Shm, None)
}

type Made = (Buffers, Generation);

fn vulkan_generation(
    frame: &ViewportFrame<'_>,
    generation: u32,
    size: (u32, u32),
) -> std::io::Result<Made> {
    let mut textures = Vec::new();
    let mut buffers = Vec::new();
    for index in 0..RING {
        let (texture, handle, desc) = crate::wgpu29::export(frame.device, size.0, size.1)?;
        textures.push(texture);
        buffers.push(crate::protocol::Buffer {
            index,
            handle: Some(handle),
            desc,
        });
    }
    clear_new(frame, &textures)?;
    Ok((
        Buffers {
            generation,
            size,
            kind: BufferKind::Vulkan,
            textures,
            maps: Vec::new(),
            upload: None,
        },
        Generation {
            generation,
            width: size.0,
            height: size.1,
            format: PixelFormat::Bgra8Unorm,
            buffers,
        },
    ))
}

/// IOSurfaces made textures on the host's Metal device, their ports sent to
/// the client over `mach` before the `Buffers` naming them.
fn iosurface_generation(
    frame: &ViewportFrame<'_>,
    generation: u32,
    size: (u32, u32),
    mach: &HostChannel,
) -> std::io::Result<Made> {
    let mut textures = Vec::new();
    let mut surfaces = Vec::new();
    let mut buffers = Vec::new();
    for index in 0..RING {
        let (texture, surface, desc) =
            crate::wgpu29::export_iosurface(frame.device, size.0, size.1)?;
        textures.push(texture);
        surfaces.push(surface);
        buffers.push(crate::protocol::Buffer {
            index,
            handle: None,
            desc,
        });
    }
    clear_new(frame, &textures)?;
    mach.send(generation, &surfaces.iter().collect::<Vec<_>>())?;
    Ok((
        Buffers {
            generation,
            size,
            kind: BufferKind::IoSurface,
            textures,
            maps: Vec::new(),
            upload: None,
        },
        Generation {
            generation,
            width: size.0,
            height: size.1,
            format: PixelFormat::Bgra8Unorm,
            buffers,
        },
    ))
}

/// A generation's new shared textures cleared, before the client has them:
/// the host's first use of each moves it out of the undefined layout (which
/// may discard) before the client draws into it, and an IOSurface's first
/// frame is transparent, not what its memory held.
fn clear_new(frame: &ViewportFrame<'_>, textures: &[wgpu::Texture]) -> std::io::Result<()> {
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("zgx surface clear"),
        });
    for texture in textures {
        let view = texture.create_view(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zgx surface clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    let submission = frame.queue.submit([encoder.finish()]);
    frame
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(2)),
        })
        .map_err(std::io::Error::other)?;
    Ok(())
}

fn shm_generation(
    frame: &ViewportFrame<'_>,
    generation: u32,
    size: (u32, u32),
) -> std::io::Result<Made> {
    let stride = size.0 * 4;
    let len = u64::from(stride) * u64::from(size.1);
    let mut maps = Vec::new();
    let mut buffers = Vec::new();
    for index in 0..RING {
        let (handle, map) = Mapping::create(len as usize)?;
        maps.push(map);
        buffers.push(crate::protocol::Buffer {
            index,
            handle: Some(handle),
            desc: BufferDesc::Shm {
                offset: 0,
                stride,
                len,
            },
        });
    }
    let upload = frame.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("zgx surface upload"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    Ok((
        Buffers {
            generation,
            size,
            kind: BufferKind::Shm,
            textures: Vec::new(),
            maps,
            upload: Some(upload),
        },
        Generation {
            generation,
            width: size.0,
            height: size.1,
            format: PixelFormat::Bgra8Unorm,
            buffers,
        },
    ))
}

/// Draw `shown` over the whole target, or clear it to transparent.
fn clear(frame: &mut ViewportFrame<'_>, shown: Option<(&Shown, &Blit)>) {
    let mut pass = frame
        .encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zgx surface"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    if let Some((shown, blit)) = shown {
        pass.set_pipeline(&blit.pipeline);
        pass.set_bind_group(0, &shown.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Blit {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("zgx surface"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zgx surface"),
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
            ],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("zgx surface"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("zgx surface"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("zgx surface"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            format,
            layout,
            sampler,
        }
    }
    fn bind(&self, device: &wgpu::Device, texture: &wgpu::Texture) -> wgpu::BindGroup {
        let view = texture.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("zgx surface"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

/// The buffer over the whole box (one triangle covering it), stretched to
/// it while a resize waits for its generation.
const BLIT: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    return textureSample(frame, linear, in.uv);
}
"#;
