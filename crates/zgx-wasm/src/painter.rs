//! The GPU view's painter: hands the guest the renderer's device and the
//! element's size, draws the newest frame it presented over the whole box
//! (a texture of the same device, sampled as it is; or its CPU pixels,
//! uploaded), and asks for the next.
use crate::{app::Inner, shared::Presented};
use std::rc::Weak;
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

pub(crate) struct Painter {
    app: Weak<Inner>,
    blit: Option<Blit>,
    device: Option<wgpu::Device>,
    /// The device's number, as the guest is told it: a texture presented
    /// for an older one is never drawn.
    generation: u64,
    shown: Option<Shown>,
    /// The CPU's pixels, uploaded.
    upload: Option<wgpu::Texture>,
}
struct Shown {
    bind: wgpu::BindGroup,
    texture: wgpu::Texture,
    /// One of the guest's frame textures, to give back once replaced.
    guests: bool,
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
            generation: 0,
            shown: None,
            upload: None,
        }
    }
    fn forget(&mut self) {
        self.blit = None;
        self.shown = None;
        self.upload = None;
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
            self.generation += 1;
            app.device(
                frame.device,
                frame.queue,
                frame.target_srgb.is_some(),
                self.generation,
            );
        }
        app.resize(frame.size.0.max(1), frame.size.1.max(1), frame.scale);
        app.tell_refresh();
        if self.blit.as_ref().is_none_or(|b| b.format != frame.format) {
            self.blit = Some(Blit::new(frame.device, frame.format));
        }
        let newest = app.link.shared.lock().unwrap().frame.take();
        let blit = self.blit.as_ref().expect("made above");
        let replaced = match newest {
            Some(Presented::Texture {
                texture,
                generation,
            }) if generation == self.generation => self.shown.replace(Shown {
                bind: blit.bind(frame.device, &texture),
                texture,
                guests: true,
            }),
            // Drawn on a device gone since: the guest draws again.
            Some(Presented::Texture { .. }) => None,
            Some(Presented::Pixels {
                pixels,
                width,
                height,
            }) => {
                let upload = match &self.upload {
                    Some(upload) if (upload.width(), upload.height()) == (width, height) => {
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
                            format: wgpu::TextureFormat::Rgba8Unorm,
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
                        guests: false,
                    })
                }
            }
            None => None,
        };
        // The frames that read it were submitted before this one, on this
        // queue: the guest's next drawing into it comes after them.
        if let Some(old) = replaced.filter(|old| old.guests) {
            app.link.shared.lock().unwrap().returned.push(old.texture);
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
