//! Each of `zgx:app/gpu`'s less common features tried once, offscreen, and
//! read back: pipeline-overridable constants, the stencil, a render bundle,
//! occlusion and timestamp queries, a pipeline made on another thread. Its
//! title says how each went (`constants=ok stencil=ok …`), for the host's
//! tests.
use zgx_app::{App, InputEvent, gpu::*, host, util};

const SIZE: u32 = 4;
const SHADER: &str = r#"
override red: f32 = 0.0;
override green: f32 = 0.0;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(red, green, 0.0, 1.0);
}
"#;

struct Features {
    results: Vec<(&'static str, String)>,
    tried: bool,
    future: Option<RenderPipelineFuture>,
}

impl App for Features {
    fn init(_info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        host::set_animating(false);
        host::request_frame();
        Ok(Features {
            results: Vec::new(),
            tried: false,
            future: None,
        })
    }
    fn frame(&mut self, _time: f64) {
        if self.tried {
            return;
        }
        self.tried = true;
        for (name, check) in [
            ("constants", constants as fn() -> Result<String, String>),
            ("stencil", stencil),
            ("bundle", bundle),
            ("occlusion", occlusion),
            ("timestamps", timestamps),
        ] {
            let result = check().unwrap_or_else(|why| format!("failed({why})"));
            self.results.push((name, result));
        }
        match module().and_then(|m| {
            Ok(create_render_pipeline_async(&pipeline(
                &m,
                &[("red", 1.)],
                None,
            ))?)
        }) {
            Ok(future) => self.future = Some(future),
            Err(why) => self.results.push(("async", format!("failed({why})"))),
        }
        self.title();
    }
    fn event(&mut self, event: InputEvent) {
        let InputEvent::PipelineReady(id) = event else {
            return;
        };
        let Some(future) = &self.future else { return };
        if future.id() != id {
            return;
        }
        let result = match future.take() {
            Some(Ok(pipeline)) => match draw_with(&pipeline, &[]) {
                Ok([255, 0, 0, 255]) => "ok".into(),
                Ok(pixel) => format!("failed({pixel:?})"),
                Err(why) => format!("failed({why})"),
            },
            Some(Err(why)) => format!("failed({why})"),
            None => "failed(ready, yet nothing to take)".into(),
        };
        self.results.push(("async", result));
        self.future = None;
        self.title();
    }
}

impl Features {
    fn title(&self) {
        let parts: Vec<String> = self
            .results
            .iter()
            .map(|(n, r)| format!("{n}={r}"))
            .collect();
        host::set_title(&parts.join(" "));
    }
}

fn module() -> Result<ShaderModule, String> {
    create_shader_module(Some("features"), SHADER)
}

/// A full-screen triangle, its constants `constants`, the stencil `stencil`.
fn pipeline<'a>(
    module: &'a ShaderModule,
    constants: &[(&str, f64)],
    stencil: Option<(StencilState, bool)>,
) -> RenderPipelineDescriptor<'a> {
    let constants: Vec<(String, f64)> =
        constants.iter().map(|(n, v)| (n.to_string(), *v)).collect();
    RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: VertexState {
            module,
            entry_point: Some("vs".into()),
            buffers: Vec::new(),
            constants: Vec::new(),
        },
        primitive: util::primitive(PrimitiveTopology::TriangleList, CullMode::None),
        depth_stencil: stencil.map(|(state, _)| DepthStencilState {
            format: TextureFormat::Depth24plusStencil8,
            depth_write_enabled: false,
            depth_compare: CompareFunction::Always,
            stencil: Some(state),
            depth_bias: 0,
            depth_bias_slope_scale: 0.,
            depth_bias_clamp: 0.,
        }),
        multisample: util::multisample(1),
        fragment: Some(FragmentState {
            module,
            entry_point: Some("fs".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8unorm,
                blend: None,
                write_mask: if stencil.is_some_and(|(_, writes)| !writes) {
                    ColorWrite::empty()
                } else {
                    ColorWrite::all()
                },
            })],
            constants,
        }),
    }
}

fn target() -> Result<(Texture, TextureView), String> {
    let texture = create_texture(&util::texture(
        "features",
        SIZE,
        SIZE,
        TextureFormat::Rgba8unorm,
        TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        1,
    ))?;
    let view = texture.create_view(None)?;
    Ok((texture, view))
}
fn color_attachment(view: &TextureView) -> RenderPassColorAttachment<'_> {
    RenderPassColorAttachment {
        view,
        resolve_target: None,
        clear: Some(Color {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 1.,
        }),
        store: StoreOp::Store,
    }
}
fn pass_descriptor(view: &TextureView) -> RenderPassDescriptor<'_> {
    RenderPassDescriptor {
        label: None,
        color_attachments: vec![Some(color_attachment(view))],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    }
}

/// `texture`'s pixels, RGBA rows of `SIZE`.
fn pixels(encoder: CommandEncoder, texture: &Texture) -> Result<Vec<u8>, String> {
    let buffer = create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * u64::from(SIZE),
        usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
    })?;
    encoder.copy_texture_to_buffer(
        &TextureCopy {
            texture,
            mip_level: 0,
            origin: Origin { x: 0, y: 0, z: 0 },
            aspect: TextureAspect::All,
        },
        &BufferCopy {
            buffer: &buffer,
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: None,
        },
        Extent {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    submit(vec![CommandEncoder::finish(encoder)?])?;
    let bytes = read_buffer(&buffer, 0, 256 * u64::from(SIZE))?;
    let mut out = Vec::new();
    for row in 0..SIZE as usize {
        out.extend_from_slice(&bytes[row * 256..row * 256 + SIZE as usize * 4]);
    }
    Ok(out)
}
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

/// What `pipeline` draws over the target, the top left pixel.
fn draw_with(pipeline: &RenderPipeline, bundles: &[&RenderBundle]) -> Result<[u8; 4], String> {
    let (texture, view) = target()?;
    let encoder = create_command_encoder(None)?;
    {
        let pass = encoder.begin_render_pass(&pass_descriptor(&view))?;
        if bundles.is_empty() {
            pass.set_pipeline(pipeline);
            pass.draw(3, 1, 0, 0);
        } else {
            pass.execute_bundles(bundles);
        }
        pass.end();
    }
    Ok(pixel(&pixels(encoder, &texture)?, 0, 0))
}

/// An override set by the pipeline reaches the shader.
fn constants() -> Result<String, String> {
    let module = module()?;
    let red = create_render_pipeline(&pipeline(&module, &[("red", 1.)], None))?;
    let green = create_render_pipeline(&pipeline(&module, &[("green", 1.)], None))?;
    match (draw_with(&red, &[])?, draw_with(&green, &[])?) {
        ([255, 0, 0, 255], [0, 255, 0, 255]) => Ok("ok".into()),
        other => Err(format!("{other:?}")),
    }
}

/// A mask drawn into the stencil on the left half only (a scissor); then
/// green where the stencil is 1: the left half is green, the right black.
fn stencil() -> Result<String, String> {
    let module = module()?;
    let mask = create_render_pipeline(&pipeline(
        &module,
        &[],
        Some((
            util::stencil(CompareFunction::Always, StencilOperation::Replace),
            false,
        )),
    ))?;
    let green = create_render_pipeline(&pipeline(
        &module,
        &[("green", 1.)],
        Some((
            util::stencil(CompareFunction::Equal, StencilOperation::Keep),
            true,
        )),
    ))?;
    let (texture, view) = target()?;
    let depth = create_texture(&util::texture(
        "stencil",
        SIZE,
        SIZE,
        TextureFormat::Depth24plusStencil8,
        TextureUsage::RENDER_ATTACHMENT,
        1,
    ))?;
    let depth_view = depth.create_view(None)?;
    let encoder = create_command_encoder(None)?;
    {
        let pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: None,
            color_attachments: vec![Some(color_attachment(&view))],
            depth_stencil_attachment: Some(RenderPassDepthAttachment {
                view: &depth_view,
                depth_clear: Some(1.),
                depth_store: StoreOp::Discard,
                depth_read_only: false,
                stencil_clear: Some(0),
                stencil_store: Some(StoreOp::Discard),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })?;
        pass.set_stencil_reference(1);
        pass.set_scissor_rect(0, 0, SIZE / 2, SIZE);
        pass.set_pipeline(&mask);
        pass.draw(3, 1, 0, 0);
        pass.set_scissor_rect(0, 0, SIZE, SIZE);
        pass.set_pipeline(&green);
        pass.draw(3, 1, 0, 0);
        pass.end();
    }
    let pixels = pixels(encoder, &texture)?;
    match (pixel(&pixels, 0, 0), pixel(&pixels, SIZE - 1, 0)) {
        ([0, 255, 0, 255], [0, 0, 0, 255]) => Ok("ok".into()),
        other => Err(format!("{other:?}")),
    }
}

/// Draws recorded once into a bundle, run by a pass.
fn bundle() -> Result<String, String> {
    let module = module()?;
    let red = create_render_pipeline(&pipeline(&module, &[("red", 1.)], None))?;
    let encoder = create_render_bundle_encoder(&RenderBundleEncoderDescriptor {
        label: None,
        color_formats: vec![Some(TextureFormat::Rgba8unorm)],
        depth_stencil: None,
        sample_count: 1,
    })?;
    encoder.set_pipeline(&red);
    encoder.draw(3, 1, 0, 0);
    let bundle = RenderBundleEncoder::finish(encoder, Some("red"))?;
    // Twice: a bundle is run as often as passes like.
    let first = draw_with(&red, &[&bundle])?;
    let again = draw_with(&red, &[&bundle])?;
    match (first, again) {
        ([255, 0, 0, 255], [255, 0, 0, 255]) => Ok("ok".into()),
        other => Err(format!("{other:?}")),
    }
}

/// The samples an occlusion query counts of a full-screen draw.
fn occlusion() -> Result<String, String> {
    let module = module()?;
    let red = create_render_pipeline(&pipeline(&module, &[("red", 1.)], None))?;
    let set = create_query_set(&QuerySetDescriptor {
        label: None,
        kind: QueryType::Occlusion,
        count: 1,
    })?;
    let (_texture, view) = target()?;
    let resolved = create_buffer(&BufferDescriptor {
        label: None,
        size: 8,
        usage: BufferUsage::QUERY_RESOLVE | BufferUsage::COPY_SRC,
    })?;
    let read = create_buffer(&BufferDescriptor {
        label: None,
        size: 8,
        usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
    })?;
    let encoder = create_command_encoder(None)?;
    {
        let mut descriptor = pass_descriptor(&view);
        descriptor.occlusion_query_set = Some(&set);
        let pass = encoder.begin_render_pass(&descriptor)?;
        pass.set_pipeline(&red);
        pass.begin_occlusion_query(0);
        pass.draw(3, 1, 0, 0);
        pass.end_occlusion_query();
        pass.end();
    }
    encoder.resolve_query_set(&set, 0, 1, &resolved, 0);
    encoder.copy_buffer_to_buffer(&resolved, 0, &read, 0, 8);
    submit(vec![CommandEncoder::finish(encoder)?])?;
    let bytes = read_buffer(&read, 0, 8)?;
    let samples = u64::from_le_bytes(bytes.try_into().unwrap());
    match samples {
        0 => Err("no samples counted".into()),
        n => Ok(format!("ok({n})")),
    }
}

/// A pass's beginning and end stamped; where the adapter has no timestamps,
/// asking for them is a plain error.
fn timestamps() -> Result<String, String> {
    if !supports_timestamps() {
        return match create_query_set(&QuerySetDescriptor {
            label: None,
            kind: QueryType::Timestamp,
            count: 2,
        }) {
            Err(_) => Ok("unsupported".into()),
            Ok(_) => Err("made without the adapter's support".into()),
        };
    }
    let module = module()?;
    let red = create_render_pipeline(&pipeline(&module, &[("red", 1.)], None))?;
    let set = create_query_set(&QuerySetDescriptor {
        label: None,
        kind: QueryType::Timestamp,
        count: 2,
    })?;
    let (_texture, view) = target()?;
    let resolved = create_buffer(&BufferDescriptor {
        label: None,
        size: 16,
        usage: BufferUsage::QUERY_RESOLVE | BufferUsage::COPY_SRC,
    })?;
    let read = create_buffer(&BufferDescriptor {
        label: None,
        size: 16,
        usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
    })?;
    let encoder = create_command_encoder(None)?;
    {
        let mut descriptor = pass_descriptor(&view);
        descriptor.timestamp_writes = Some(PassTimestampWrites {
            query_set: &set,
            beginning_index: Some(0),
            end_index: Some(1),
        });
        let pass = encoder.begin_render_pass(&descriptor)?;
        pass.set_pipeline(&red);
        pass.draw(3, 1, 0, 0);
        pass.end();
    }
    encoder.resolve_query_set(&set, 0, 2, &resolved, 0);
    encoder.copy_buffer_to_buffer(&resolved, 0, &read, 0, 16);
    submit(vec![CommandEncoder::finish(encoder)?])?;
    let bytes = read_buffer(&read, 0, 16)?;
    let begin = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let end = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if begin == 0 || end < begin {
        return Err(format!("{begin}..{end}"));
    }
    let ns = (end - begin) as f64 * f64::from(get_timestamp_period());
    Ok(format!("ok({ns:.0}ns)"))
}

zgx_app::app!(Features);
