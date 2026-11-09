//! The GPU view's painter: tells the guest the adapter zgx draws with (it
//! opens a device of its own there), makes the ring of textures the guest's
//! device draws into and shares them, draws the newest frame presented over
//! the whole box (a shared texture sampled as it is, or pixels uploaded),
//! gives the one it replaced back once zgx's GPU is done reading it, and
//! asks for the next frame.
use crate::{
    app::Inner,
    device::{HostGpu, Sharing},
    runtime::ToGuest,
    shared::Presented,
};
use std::{
    rc::Weak,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

/// How long a new size holds before a new ring is made for it (a window
/// being dragged shows the last frame stretched until then).
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(120);

pub(crate) struct Painter {
    app: Weak<Inner>,
    blit: Option<Blit>,
    device: Option<wgpu::Device>,
    /// The adapter the guest was told of.
    host: Option<HostGpu>,
    sharing: Sharing,
    ring: Option<Ring>,
    generation: u32,
    /// The guest was told it shares nothing (and reads its frames back).
    told_readback: bool,
    /// When the box first had a size its ring isn't.
    resizing: Option<Instant>,
    shown: Option<Shown>,
    /// Pixels from the CPU (or read back), uploaded.
    upload: Option<wgpu::Texture>,
}
/// zgx's shared textures at one size.
struct Ring {
    generation: u32,
    size: (u32, u32),
    textures: Vec<wgpu::Texture>,
}
struct Shown {
    bind: wgpu::BindGroup,
    texture: wgpu::Texture,
    /// A ring's texture (generation, index), to give back once replaced.
    shared: Option<(u32, u32)>,
}
struct Blit {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Painter {
    pub(crate) fn new(app: Weak<Inner>) -> Self {
        Self {
            app,
            blit: None,
            device: None,
            host: None,
            sharing: Sharing::Readback,
            ring: None,
            generation: 0,
            told_readback: false,
            resizing: None,
            shown: None,
            upload: None,
        }
    }
    fn forget(&mut self) {
        self.blit = None;
        self.shown = None;
        self.upload = None;
        self.ring = None;
        self.resizing = None;
        self.told_readback = false;
    }
    /// A ring at `size` for the guest: at once for the first, else once a
    /// new size has held for `RESIZE_DEBOUNCE`.
    fn size_ring(&mut self, app: &Inner, frame: &ViewportFrame<'_>, size: (u32, u32)) {
        let sharing = if app.unshared.get() {
            Sharing::Readback
        } else {
            self.sharing
        };
        if sharing == Sharing::Readback {
            if !self.told_readback {
                self.ring = None;
                self.told_readback = true;
                self.generation += 1;
                app.send(ToGuest::Buffers {
                    generation: self.generation,
                    buffers: Vec::new(),
                    cleared: None,
                });
            }
            return;
        }
        match &self.ring {
            Some(ring) if ring.size == size => {
                self.resizing = None;
                return;
            }
            Some(_) => {
                let since = *self.resizing.get_or_insert(frame.time);
                let held = frame.time.saturating_duration_since(since);
                if held < RESIZE_DEBOUNCE {
                    app.invalidate_after(RESIZE_DEBOUNCE - held);
                    return;
                }
            }
            None => {}
        }
        self.resizing = None;
        match crate::device::export_ring(frame.device, frame.queue, sharing, size) {
            Ok((textures, buffers, cleared)) => {
                self.generation += 1;
                self.ring = Some(Ring {
                    generation: self.generation,
                    size,
                    textures,
                });
                app.send(ToGuest::Buffers {
                    generation: self.generation,
                    buffers,
                    cleared,
                });
            }
            Err(error) => {
                eprintln!("zgx-wasm: textures to share can't be made ({error}): read back instead");
                self.sharing = Sharing::Readback;
            }
        }
    }
}

impl Viewport for Painter {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let Some(app) = self.app.upgrade().filter(|app| !app.revoked.get()) else {
            self.forget();
            draw(frame, None);
            return;
        };
        app.scale.set(frame.scale);
        app.painted.set(true);
        if self.device.as_ref() != Some(frame.device) {
            self.forget();
            self.device = Some(frame.device.clone());
            self.sharing = crate::device::sharing_of(frame.device);
            let host = HostGpu::of(frame.device);
            // The same adapter again (a device made anew): the guest's device
            // stays; only the ring is new.
            if self.host.as_ref() != Some(&host) {
                app.send(ToGuest::Device(host.clone()));
                self.host = Some(host);
            }
        }
        let size = (frame.size.0.max(1), frame.size.1.max(1));
        self.size_ring(&app, frame, size);
        app.resize(size.0, size.1, frame.scale);
        app.tell_refresh();
        if self.blit.as_ref().is_none_or(|b| b.format != frame.format) {
            self.blit = Some(Blit::new(frame.device, frame.format));
        }
        let newest = app.link.shared.lock().unwrap().frame.take();
        let blit = self.blit.as_ref().expect("made above");
        let replaced = match newest {
            Some(Presented::Shared { generation, index }) => match &self.ring {
                Some(ring) if ring.generation == generation => {
                    let texture = ring.textures[index as usize].clone();
                    self.shown.replace(Shown {
                        bind: blit.bind(frame.device, &texture),
                        texture,
                        shared: Some((generation, index)),
                    })
                }
                // A ring gone since: the guest draws into the new one.
                _ => None,
            },
            Some(Presented::Pixels {
                pixels,
                width,
                height,
                bgra,
            }) => {
                let format = if bgra {
                    wgpu::TextureFormat::Bgra8Unorm
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                };
                let upload = match &self.upload {
                    Some(upload)
                        if (upload.width(), upload.height(), upload.format())
                            == (width, height, format) =>
                    {
                        upload.clone()
                    }
                    _ => {
                        let upload = frame.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("zgx-wasm pixels"),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        self.upload = Some(upload.clone());
                        upload
                    }
                };
                frame.queue.write_texture(
                    upload.as_image_copy(),
                    &pixels,
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
                if self.shown.as_ref().is_some_and(|s| s.texture == upload) {
                    None
                } else {
                    self.shown.replace(Shown {
                        bind: blit.bind(frame.device, &upload),
                        texture: upload,
                        shared: None,
                    })
                }
            }
            None => None,
        };
        // Given back once zgx's GPU has read it: the frames that did were
        // submitted before this one, on its queue; the guest's device is
        // another and orders nothing after them.
        if let Some(released) = replaced.and_then(|old| old.shared) {
            let (link, awaiting) = (app.link.clone(), app.awaiting.clone());
            awaiting.fetch_add(1, Ordering::AcqRel);
            frame.queue.on_submitted_work_done(move || {
                link.shared.lock().unwrap().released.push(released);
                awaiting.fetch_sub(1, Ordering::AcqRel);
            });
            app.poll_until_released(frame.device);
        }
        draw(frame, self.shown.as_ref().zip(self.blit.as_ref()));
        app.ask_frame(frame.time);
    }
    fn device_changed(&mut self) {
        self.forget();
        self.device = None;
    }
}

/// `shown` over the whole target, or the target cleared to transparent.
fn draw(frame: &mut ViewportFrame<'_>, shown: Option<(&Shown, &Blit)>) {
    let mut pass = frame
        .encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zgx-wasm"),
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
            label: Some("zgx-wasm"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zgx-wasm"),
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
            label: Some("zgx-wasm"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("zgx-wasm"),
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
            label: Some("zgx-wasm"),
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
        // Viewed as stored (unorm): what the guest wrote is what shows.
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(texture.format().remove_srgb_suffix()),
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("zgx-wasm"),
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

/// The frame over the whole box (one triangle covering it), stretched to
/// it while a new size's first frame is drawn.
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
