//! What a layer shader reads besides its layer (`FillShader::with_inputs`):
//! passes drawn before each frame showing it, pictures, and tagged nodes'
//! boxes.
//!
//! A shader with inputs is one running effect, so its `Chain` lives and dies
//! with its id (`drop_unheld_shaders`), and its draws never batch with
//! another's: a shader's id is what batches. Nothing here runs for any other
//! shader. Passes are full-screen fragment stages, which every backend has,
//! so a chain draws the same everywhere.
use crate::{GpuRenderer, compile};
use std::sync::Arc;
use zgx::decoration::{FillShader, PassFormat, PassSize, ShaderInputs};

/// The group a layer shader's inputs bind at.
pub(crate) const GROUP: u32 = 3;
/// The boxes a shader can read: past these, the first so many.
const MOST_RECTS: usize = 64;
/// Bytes of one step's uniforms: `wgpu`'s largest offset alignment.
const STEP_BYTES: u64 = 256;
/// The textures one effect may hold, all told.
const MOST_BYTES: u64 = 256 * 1024 * 1024;
/// A frame's largest step of time a pass sees, in seconds: what a window
/// hidden for a minute shouldn't feed its simulation.
const LONGEST_STEP: f32 = 1. / 20.;
/// What declares the inputs in a module: `zgx_<name>(uv)`, `_texel`, `_size`,
/// and the boxes.
fn declarations(group: u32, names: &[&str]) -> String {
    let mut out = format!(
        "@group({group}) @binding(0) var zgx_input_sampler:sampler;
@group({group}) @binding(1) var<storage,read> zgx_rects:array<vec4<f32>>;
// How many boxes the tag has, and box `i`: x, y, width, height in window
// points, and its corner radii.
fn zgx_rect_count()->u32 {{ return u32(zgx_rects[0].x); }}
fn zgx_rect(i:u32)->vec4<f32> {{ return zgx_rects[1u+2u*i]; }}
fn zgx_rect_radii(i:u32)->vec4<f32> {{ return zgx_rects[2u+2u*i]; }}
// Signed distance in points from `p` (window points) to box `i`'s edge:
// negative inside.
fn zgx_rect_distance(p:vec2<f32>,i:u32)->f32 {{
 let b=zgx_rect(i); let c=b.xy+b.zw*.5; let q=p-c; let k=zgx_rect_radii(i);
 let r=select(select(k.w,k.z,q.x>0.),select(k.x,k.y,q.x>0.),q.y<0.);
 let d=abs(q)-b.zw*.5+vec2(r);
 return length(max(d,vec2(0.)))+min(max(d.x,d.y),0.)-r;
}}
"
    );
    for (index, name) in names.iter().enumerate() {
        let (binding, at) = (index + 2, format!("zgx_input_{index}"));
        out += &format!(
            "@group({group}) @binding({binding}) var {at}:texture_2d<f32>;
fn zgx_{name}(uv:vec2<f32>)->vec4<f32> {{ return textureSampleLevel({at},zgx_input_sampler,uv,0.); }}
fn zgx_{name}_texel(xy:vec2<i32>)->vec4<f32> {{ return textureLoad({at},xy,0); }}
fn zgx_{name}_size()->vec2<f32> {{ return vec2<f32>(textureDimensions({at})); }}
"
        );
    }
    out
}
/// The input's names, passes first, in binding order.
fn names(inputs: &ShaderInputs) -> Vec<&str> {
    (inputs.passes.iter().map(|pass| pass.name.as_str()))
        .chain(inputs.images.iter().map(|(name, _)| name.as_str()))
        .chain(inputs.views.iter().map(|(name, _)| name.as_str()))
        .collect()
}
/// How many textures `inputs` binds.
pub(crate) fn textures(inputs: &ShaderInputs) -> u32 {
    (inputs.passes.len() + inputs.images.len() + inputs.views.len()) as u32
}
/// The module a pass's `run` becomes.
fn pass_source(user: &str, inputs: &ShaderInputs) -> String {
    format!(
        "struct PassInput {{ uv:vec2<f32>, pixel:vec2<f32>, size:vec2<f32>, time:f32, dt:f32, step:u32, frame:u32, scale:f32, area:vec4<f32>, params:array<vec4<f32>,4> }}
struct ZgxPass {{ size:vec2<f32>, time:f32, dt:f32, area:vec4<f32>, step:f32, frame:f32, scale:f32, pad:f32, params:array<vec4<f32>,4> }}
@group(1) @binding(0) var<uniform> zgx_pass:ZgxPass;
{}
@vertex fn zgx_pass_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {{
 return vec4(vec2(f32(i&1u)*4.-1.,f32(i&2u)*2.-1.),0.,1.);
}}
@fragment fn zgx_pass_fragment(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {{
 let u=zgx_pass;
 return run(PassInput(at.xy/u.size,at.xy,u.size,u.time,u.dt,u32(u.step),u32(u.frame),u.scale,u.area,u.params));
}}
{user}",
        declarations(0, &names(inputs)),
    )
}
/// A layer shader's module with its inputs.
fn composite_source(shader: &FillShader, inputs: &ShaderInputs) -> String {
    crate::filter_source(&format!(
        "{}{}",
        declarations(GROUP, &names(inputs)),
        shader.source()
    ))
}
/// What's wrong with a shader and its passes, as an app or script shows it.
pub fn check(user: &str, inputs: &ShaderInputs) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for name in names(inputs) {
        let identifier = name.starts_with(|c: char| c.is_ascii_alphabetic())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !identifier {
            return Err(format!("`{name}` isn't a name a shader can use"));
        }
        if !seen.insert(name) {
            return Err(format!("two inputs are named `{name}`"));
        }
    }
    let valid = |source: &str| {
        let module = wgpu::naga::front::wgsl::parse_str(source).map_err(|e| {
            let at = e.location(source).map_or(0, |at| at.line_number as usize);
            let before = source
                .find("\u{1}")
                .map_or(0, |at| source[..at].matches('\n').count());
            format!("line {}: {}", at.saturating_sub(before), e.message())
        })?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map(|_| ())
        .map_err(|e| e.as_inner().to_string())
    };
    for pass in &inputs.passes {
        valid(&pass_source(&format!("//\u{1}\n{}", pass.source), inputs))
            .map_err(|e| format!("pass `{}`: {e}", pass.name))?;
    }
    let declared = declarations(GROUP, &names(inputs));
    valid(&crate::filter_source(&format!("{declared}//\u{1}\n{user}")))
}

struct Pass {
    pipeline: compile::Compiled<wgpu::RenderPipeline>,
    format: wgpu::TextureFormat,
    /// One texture, or two to flip between. `views[current]` is its last
    /// result.
    views: Vec<wgpu::TextureView>,
    current: usize,
    size: (u32, u32),
    /// It has drawn since its textures were made (a `once` pass's end).
    drawn: bool,
}
/// One layer shader's inputs on one renderer.
pub(crate) struct Chain {
    device: wgpu::Device,
    /// The layer shaders' groups 0 and 1.
    layouts: (wgpu::BindGroupLayout, wgpu::BindGroupLayout),
    /// Group 0 of a pass, and `GROUP` of the shader: the inputs.
    inputs_layout: wgpu::BindGroupLayout,
    passes: Vec<Pass>,
    images: Vec<wgpu::TextureView>,
    /// Views' frames, and the size they're at.
    views: Vec<(u64, (u32, u32), Option<wgpu::TextureView>)>,
    /// Bound where a pass reads itself and keeps no last result.
    nothing: wgpu::TextureView,
    sampler: wgpu::Sampler,
    rects: wgpu::Buffer,
    rects_held: Vec<(zgx::scene::Rect, [f32; 4])>,
    uniforms: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
    /// What the layer shader binds at `GROUP`: the passes' latest.
    bound: Option<wgpu::BindGroup>,
    /// The shader's time when it last ran, and frames so far.
    last: Option<f32>,
    frames: u32,
    /// The pass-time it last ran at: a second node showing the shader, or
    /// the same frame again, runs nothing.
    last_frame: u64,
}

fn target_format(format: PassFormat) -> wgpu::TextureFormat {
    match format {
        PassFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
        PassFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
    }
}
fn picture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height): (u32, u32),
    pixels: &[u8],
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shader input"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

impl Chain {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layouts: (wgpu::BindGroupLayout, wgpu::BindGroupLayout),
        inputs: &ShaderInputs,
    ) -> Self {
        let count = textures(inputs) as usize;
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        entries.extend((0..count).map(|index| wgpu::BindGroupLayoutEntry {
            binding: index as u32 + 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }));
        let inputs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader inputs"),
            entries: &entries,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader pass"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(112),
                },
                count: None,
            }],
        });
        let steps: u64 = (inputs.passes.iter())
            .map(|pass| u64::from(pass.steps.max(1)))
            .sum();
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader pass uniforms"),
            size: steps.max(1) * STEP_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader pass uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(112),
                }),
            }],
        });
        let rects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader boxes"),
            size: (1 + 2 * MOST_RECTS as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader pass"),
            bind_group_layouts: &[Some(&inputs_layout), Some(&uniform_layout)],
            immediate_size: 0,
        });
        let passes = inputs
            .passes
            .iter()
            .map(|pass| {
                let (device, layout, format) =
                    (device.clone(), layout.clone(), target_format(pass.format));
                let source = pass_source(&pass.source, inputs);
                let slot = compile::spawn(move || {
                    crate::fill_validates(&source)
                        .then(|| {
                            crate::shader_pipeline(&device, || {
                                let module =
                                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                        label: Some("shader pass"),
                                        source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
                                    });
                                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                    label: Some("shader pass"),
                                    layout: Some(&layout),
                                    vertex: wgpu::VertexState {
                                        module: &module,
                                        entry_point: Some("zgx_pass_vertex"),
                                        compilation_options: Default::default(),
                                        buffers: &[],
                                    },
                                    primitive: Default::default(),
                                    depth_stencil: None,
                                    multisample: Default::default(),
                                    fragment: Some(wgpu::FragmentState {
                                        module: &module,
                                        entry_point: Some("zgx_pass_fragment"),
                                        compilation_options: Default::default(),
                                        targets: &[Some(format.into())],
                                    }),
                                    multiview_mask: None,
                                    cache: None,
                                })
                            })
                        })
                        .flatten()
                });
                Pass {
                    pipeline: compile::Compiled::Pending(slot),
                    format,
                    views: Vec::new(),
                    current: 0,
                    size: (0, 0),
                    drawn: false,
                }
            })
            .collect();
        let images = (inputs.images.iter())
            .map(|(_, image)| {
                let pixels = image.pixels_now();
                picture(device, queue, (image.width(), image.height()), &pixels)
            })
            .collect();
        Self {
            device: device.clone(),
            layouts,
            inputs_layout,
            passes,
            images,
            views: (inputs.views.iter())
                .map(|(_, view)| (*view, (0, 0), None))
                .collect(),
            nothing: picture(device, queue, (1, 1), &[0; 4]),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("shader inputs"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            rects,
            rects_held: Vec::new(),
            uniforms,
            uniform_bind,
            bound: None,
            last: None,
            frames: 0,
            last_frame: 0,
        }
    }
    /// The layout and module a layer shader draws with, unblended.
    pub(crate) fn quads_layout(&self, shader: &FillShader) -> (wgpu::PipelineLayout, String) {
        let (texture, viewport) = (self.layouts.0.clone(), self.layouts.1.clone());
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("layer shader with inputs"),
                bind_group_layouts: &[
                    Some(&texture),
                    Some(&viewport),
                    None,
                    Some(&self.inputs_layout),
                ],
                immediate_size: 0,
            });
        (
            layout,
            composite_source(shader, shader.inputs().expect("it has a chain")),
        )
    }
    /// Blended, reading the backdrop in group 2.
    pub(crate) fn blend_layout(
        &self,
        shader: &FillShader,
        backdrop: &wgpu::BindGroupLayout,
    ) -> (wgpu::PipelineLayout, String) {
        let (texture, viewport) = (self.layouts.0.clone(), self.layouts.1.clone());
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("blended layer shader with inputs"),
                bind_group_layouts: &[
                    Some(&texture),
                    Some(&viewport),
                    Some(backdrop),
                    Some(&self.inputs_layout),
                ],
                immediate_size: 0,
            });
        (
            layout,
            composite_source(shader, shader.inputs().expect("it has a chain")),
        )
    }
    /// Bindings with each pass's latest result, but `writing`'s own texture.
    fn bind_inputs(&self, writing: Option<usize>) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: self.rects.as_entire_binding(),
            },
        ];
        for (index, pass) in self.passes.iter().enumerate() {
            let view = match Some(index) == writing && pass.views.len() < 2 {
                true => &self.nothing,
                false => pass.views.get(pass.current).unwrap_or(&self.nothing),
            };
            entries.push(wgpu::BindGroupEntry {
                binding: index as u32 + 2,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        for (index, view) in self.images.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: (self.passes.len() + index) as u32 + 2,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let first = self.passes.len() + self.images.len();
        for (index, (_, _, view)) in self.views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: (first + index) as u32 + 2,
                resource: wgpu::BindingResource::TextureView(
                    view.as_ref().unwrap_or(&self.nothing),
                ),
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader inputs"),
            layout: &self.inputs_layout,
            entries: &entries,
        })
    }
}

/// Make `(pass, chain)`'s textures for `size`, if it hasn't them. How many
/// bytes of them it holds.
fn sized(device: &wgpu::Device, pass: &mut Pass, size: (u32, u32), both: bool) {
    if pass.size == size && !pass.views.is_empty() {
        return;
    }
    let make = || {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shader pass"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: pass.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    pass.views = (0..if both { 2 } else { 1 }).map(|_| make()).collect();
    pass.current = 0;
    pass.size = size;
    pass.drawn = false;
}

impl GpuRenderer {
    /// A frame's layer shaders with inputs, before anything draws them:
    /// each one's pipeline started with its inputs' layout (so the frame's
    /// own preparation finds it as any layer shader's), and its passes run.
    /// One whose passes can't run yet has its pipeline put aside for the
    /// frame, so its layer composites as painted.
    #[cold]
    #[inline(never)]
    pub(crate) fn prepare_inputs(&mut self, scene: &zgx::scene::Scene) {
        for (reading, id, compiled) in std::mem::take(&mut self.parked_inputs) {
            let pipelines = match reading {
                true => &mut self.blend_pipelines,
                false => &mut self.filter_pipelines,
            };
            pipelines.insert(id, compiled);
        }
        let viewport = zgx::scene::Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        for (node, bounds) in scene.layer_input_boxes() {
            let Some(layer) = scene.layer_shader(node).cloned() else {
                continue;
            };
            let Some(shader) = layer.shader.clone() else {
                continue;
            };
            let Some(inputs) = shader.inputs() else {
                continue;
            };
            let reading = shader.reads_backdrop();
            let id = shader.id();
            self.start_inputs(&shader, inputs);
            // Out of the window: no passes, as no frame shows it.
            if !bounds.intersects(viewport) {
                continue;
            }
            if !self.run_passes(scene, bounds, &shader, &layer) {
                let pipelines = match reading {
                    true => &mut self.blend_pipelines,
                    false => &mut self.filter_pipelines,
                };
                if let Some(compiled) = pipelines.get_mut(&id) {
                    let compiled = std::mem::replace(compiled, compile::Compiled::Done(None));
                    self.parked_inputs.push((reading, id, compiled));
                }
            }
        }
    }
    /// `shader`'s layer pipeline with its inputs' bindings, started if it
    /// isn't yet: by the frame's pre-pass, or as a script declares it.
    pub(crate) fn start_inputs(&mut self, shader: &Arc<FillShader>, inputs: &ShaderInputs) {
        let reading = shader.reads_backdrop();
        let id = shader.id();
        self.filter_shaders.insert(id, shader.clone());
        let started = match reading {
            true => self.blend_pipelines.contains_key(&id),
            false => self.filter_pipelines.contains_key(&id),
        };
        if !started {
            self.start_with_inputs(shader, inputs, reading);
        }
    }
    /// Start compiling `shader`'s layer pipeline (blended, if it reads the
    /// backdrop) with its inputs' bindings.
    fn start_with_inputs(
        &mut self,
        shader: &Arc<FillShader>,
        inputs: &ShaderInputs,
        reading: bool,
    ) {
        let apps = true;
        let device = self.device.clone();
        let backend = self.info.backend;
        let textures = textures(inputs);
        let (layout, source, entry) = match reading {
            true => {
                let backdrop = self.blend_layouts().0.clone();
                let (layout, source) = self
                    .chain(shader.id(), inputs)
                    .blend_layout(shader, &backdrop);
                (layout, source, "zgx_filter_blended")
            }
            false => {
                let (layout, source) = self.chain(shader.id(), inputs).quads_layout(shader);
                (layout, source, "zgx_filter")
            }
        };
        let app_layout = crate::AppLayout::Inputs {
            blend: reading,
            textures,
        };
        let slot = compile::spawn(move || {
            crate::fill_validates(&source)
                .then(|| {
                    Self::app_built(
                        &device,
                        backend,
                        apps,
                        &source,
                        entry,
                        app_layout,
                        |vertex, fragment| match reading {
                            true => {
                                crate::blend_pipeline(&device, vertex, fragment, &layout, entry)
                            }
                            false => {
                                crate::fill_pipeline(&device, vertex, fragment, &layout, entry)
                            }
                        },
                    )
                })
                .flatten()
        });
        let pipelines = match reading {
            true => &mut self.blend_pipelines,
            false => &mut self.filter_pipelines,
        };
        pipelines.insert(shader.id(), compile::Compiled::Pending(slot));
    }
    /// `id`'s chain, made if new.
    pub(crate) fn chain(&mut self, id: u64, inputs: &ShaderInputs) -> &Chain {
        let (device, queue) = (&self.device, &self.queue);
        let inner = &self.context.inner;
        self.chains.entry(id).or_insert_with(|| {
            let layouts = (inner.texture_layout.clone(), inner.viewport_layout.clone());
            Chain::new(device, queue, layouts, inputs)
        })
    }
    /// Bind what the layer shader `shading` draws with at `GROUP`, if it
    /// has inputs. Out of line: only frames with such a shader call it.
    #[cold]
    #[inline(never)]
    pub(crate) fn bind_inputs(&self, pass: &mut wgpu::RenderPass<'_>, shading: crate::Shading) {
        let (crate::Shading::Filter(id) | crate::Shading::FilterBlend(id)) = shading else {
            return;
        };
        if let Some(bound) = self.chains.get(&id).and_then(|chain| chain.bound.as_ref()) {
            pass.set_bind_group(GROUP, bound, &[]);
        }
    }
    /// Run `shader`'s passes for this frame, once however many nodes show it,
    /// ahead of the frame sampling them. Whether it can draw: its passes have
    /// compiled (until then its layer composites as painted, as a shader's
    /// does while it compiles) and none failed.
    #[cold]
    #[inline(never)]
    pub(crate) fn run_passes(
        &mut self,
        scene: &zgx::scene::Scene,
        area: zgx::scene::Rect,
        shader: &Arc<FillShader>,
        layer: &zgx::scene::LayerShader,
    ) -> bool {
        let Some(inputs) = shader.inputs() else {
            return true;
        };
        let wait = self.waits_for_shaders();
        let frame = self.image_frame;
        let scale = self.scale;
        let (device, queue) = (self.device.clone(), self.queue.clone());
        self.chain(shader.id(), inputs);
        let chain = self.chains.get_mut(&shader.id()).expect("just made");
        let mut compiling = false;
        for pass in &mut chain.passes {
            match pass.pipeline.ready(wait) {
                Some(true) => {}
                Some(false) => return false,
                None => compiling = true,
            }
        }
        if compiling {
            self.images_awaited = true;
            return false;
        }
        if chain.last_frame == frame && chain.bound.is_some() {
            return true;
        }
        chain.last_frame = frame;
        // Views' frames, at the box's size, by their painters.
        let box_size = (
            ((area.width * scale).ceil() as u32).max(1),
            ((area.height * scale).ceil() as u32).max(1),
        );
        for at in 0..chain.views.len() {
            let (view, size, _) = &chain.views[at];
            if *size != box_size || chain.views[at].2.is_none() {
                let made = device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("shader input view"),
                        size: wgpu::Extent3d {
                            width: box_size.0,
                            height: box_size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: crate::FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default());
                chain.views[at] = (*view, box_size, Some(made));
            }
        }
        let painting: Vec<_> = (chain.views.iter())
            .filter_map(|(view, _, target)| Some((*view, target.clone()?)))
            .collect();
        // Textures, at this frame's size.
        let max = device.limits().max_texture_dimension_2d;
        let mut bytes = 0u64;
        for (pass, declared) in chain.passes.iter_mut().zip(&inputs.passes) {
            let size = match declared.size {
                PassSize::Pixels(width, height) => (width.clamp(1, max), height.clamp(1, max)),
                PassSize::OfBox(of) => (
                    ((area.width * scale * of).ceil() as u32).clamp(1, max),
                    ((area.height * scale * of).ceil() as u32).clamp(1, max),
                ),
            };
            let texel = match declared.format {
                PassFormat::Rgba8 => 4,
                PassFormat::Rgba16Float => 8,
            };
            let count = if declared.feedback { 2 } else { 1 };
            bytes += u64::from(size.0) * u64::from(size.1) * texel * count;
            if bytes > MOST_BYTES {
                eprintln!("zgx: a shader's passes exceed {MOST_BYTES} bytes: it draws nothing");
                return false;
            }
            sized(&device, pass, size, declared.feedback);
        }
        // The boxes it reads.
        if let Some(tag) = inputs.geometry {
            let mut held = std::mem::take(&mut chain.rects_held);
            held.clear();
            scene.geometry_of(tag, &mut held);
            held.truncate(MOST_RECTS);
            let mut data = Vec::with_capacity(1 + held.len() * 2);
            data.push([held.len() as f32, 0., 0., 0.]);
            for (rect, radii) in &held {
                data.push([rect.x, rect.y, rect.width, rect.height]);
                data.push(*radii);
            }
            queue.write_buffer(&chain.rects, 0, bytemuck::cast_slice(&data));
            chain.rects_held = held;
        }
        // Uniforms for every step that runs.
        let now = layer.uniforms.time;
        let dt = chain
            .last
            .map_or(0., |last| (now - last).clamp(0., LONGEST_STEP));
        chain.last = Some(now);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shader passes"),
        });
        let uniforms = layer.uniforms;
        let mut offset = 0u64;
        let mut stream = vec![0u8; chain.uniforms.size() as usize];
        for at in 0..chain.passes.len() {
            let declared = &inputs.passes[at];
            let steps = declared.steps.max(1);
            if declared.once && chain.passes[at].drawn {
                offset += u64::from(steps) * STEP_BYTES;
                continue;
            }
            for step in 0..steps {
                let pass = &chain.passes[at];
                let [width, height] = [pass.size.0 as f32, pass.size.1 as f32];
                let mut block = [0f32; 28];
                block[..12].copy_from_slice(&[
                    width,
                    height,
                    uniforms.time,
                    dt,
                    area.x,
                    area.y,
                    area.width,
                    area.height,
                    step as f32,
                    chain.frames as f32,
                    scale,
                    0.,
                ]);
                for (index, value) in uniforms.params.iter().flatten().enumerate() {
                    block[12 + index] = *value;
                }
                let at_byte = offset as usize;
                stream[at_byte..at_byte + 112].copy_from_slice(bytemuck::cast_slice(&block));
                let inputs_bind = chain.bind_inputs(Some(at));
                let pass = &chain.passes[at];
                let target = &pass.views[if pass.views.len() > 1 {
                    1 - pass.current
                } else {
                    0
                }];
                {
                    let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shader pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if let Some(pipeline) = pass.pipeline.get() {
                        render.set_pipeline(pipeline);
                        render.set_bind_group(0, &inputs_bind, &[]);
                        render.set_bind_group(1, &chain.uniform_bind, &[offset as u32]);
                        render.draw(0..3, 0..1);
                    }
                }
                offset += STEP_BYTES;
                let pass = &mut chain.passes[at];
                if pass.views.len() > 1 {
                    pass.current = 1 - pass.current;
                }
                pass.drawn = true;
            }
        }
        queue.write_buffer(&chain.uniforms, 0, &stream);
        chain.frames = chain.frames.wrapping_add(1);
        chain.bound = Some(chain.bind_inputs(None));
        queue.submit([encoder.finish()]);
        for (view, target) in painting {
            if self
                .paint((view, shader.id()), box_size, &target, None, crate::FORMAT)
                .is_err()
            {
                return false;
            }
        }
        true
    }
}
