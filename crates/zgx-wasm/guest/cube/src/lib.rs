//! A lit 3D scene drawn with WebGPU on a device of its own: a cube turning
//! inside a ring of cubes, mirrored in a glossy floor (the stencil keeps the
//! reflection to the floor; its pipeline is made on another thread), with a
//! wall of a cube for each character typed. The cubes are a render bundle,
//! recorded again only as their count changes; where the GPU stamps
//! timestamps, the title says how long the pass took. Drag to orbit (or press the right button to lock the
//! pointer and look about; Escape lets go), wheel or arrows to zoom, Space
//! pauses, R reverses, C changes colours, 1 to 4 set the ring's cubes. It
//! animates by the display's clock, so it turns as fast at 60 Hz as at
//! 144; its title says what it heard and the rate it draws at.
use zgx_app::{
    App, Button, Cursor, InputEvent,
    gpu::{self, *},
    host, util,
};

const SAMPLES: u32 = 4;
const DEPTH: TextureFormat = TextureFormat::Depth24plusStencil8;
const MAX_INSTANCES: usize = 1 + 24 + 1 + 64;
const SHADER: &str = r#"
struct Scene { view_proj: mat4x4<f32>, eye: vec4<f32>, light: vec4<f32>, params: vec4<f32> }
@group(0) @binding(0) var<uniform> scene: Scene;
struct In {
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
    @location(2) m0: vec4<f32>, @location(3) m1: vec4<f32>, @location(4) m2: vec4<f32>, @location(5) m3: vec4<f32>,
    @location(6) color: vec4<f32>,
}
struct Out {
    @builtin(position) clip: vec4<f32>, @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>,
}
@vertex fn vs(v: In) -> Out {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let world = model * vec4<f32>(v.position, 1.0);
    var out: Out;
    out.clip = scene.view_proj * world;
    out.world = world.xyz;
    out.normal = normalize((model * vec4<f32>(v.normal, 0.0)).xyz);
    out.color = v.color;
    return out;
}
@fragment fn fs(i: Out) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    let l = normalize(-scene.light.xyz);
    let v = normalize(scene.eye.xyz - i.world);
    let h = normalize(l + v);
    let diffuse = max(dot(n, l), 0.0);
    let specular = pow(max(dot(n, h), 0.0), 48.0) * 0.6 * i.color.a;
    let sky = mix(vec3<f32>(0.10, 0.08, 0.07), vec3<f32>(0.22, 0.28, 0.40), n.y * 0.5 + 0.5);
    var albedo = i.color.rgb;
    // The floor (the least shiny): a checker fading into the fog.
    if (i.color.a < 0.2) {
        let cell = floor(i.world.xz * 1.25);
        let check = abs(cell.x + cell.y) % 2.0;
        albedo = albedo * mix(0.75, 1.15, check);
    }
    var c = albedo * (sky * 0.7 + diffuse * vec3<f32>(1.0, 0.93, 0.82) * 1.6) + vec3<f32>(specular);
    let fog = clamp((length(scene.eye.xyz - i.world) - 8.0) / 16.0, 0.0, 1.0);
    c = mix(c, vec3<f32>(0.012, 0.014, 0.022), fog);
    // Reflections are dimmer than what they mirror.
    c = c * mix(1.0, 0.5, scene.params.y);
    c = vec3<f32>(1.0) - exp(-c * 1.3);
    // The floor lets its reflections show through.
    let alpha = select(1.0, 0.72, i.color.a < 0.2);
    return vec4<f32>(pow(c, vec3<f32>(1.0 / 2.2)), alpha);
}
"#;

const PALETTES: [[[f32; 3]; 3]; 4] = [
    [[0.95, 0.45, 0.20], [0.20, 0.55, 0.95], [0.95, 0.80, 0.25]],
    [[0.30, 0.85, 0.55], [0.85, 0.30, 0.65], [0.95, 0.95, 0.95]],
    [[0.90, 0.20, 0.25], [0.95, 0.60, 0.15], [0.30, 0.25, 0.85]],
    [[0.55, 0.75, 0.95], [0.25, 0.35, 0.55], [0.90, 0.90, 0.70]],
];

struct Resources {
    /// The cubes; the floor, blended over their reflections; the floor
    /// marked in the stencil; the cubes mirrored in it, only where it is
    /// marked (made on another thread: none until it's ready).
    pipeline: RenderPipeline,
    floor: RenderPipeline,
    mask: RenderPipeline,
    reflect: Option<RenderPipeline>,
    reflect_future: Option<RenderPipelineFuture>,
    bind_group: BindGroup,
    mirrored_group: BindGroup,
    uniforms: gpu::Buffer,
    mirrored: gpu::Buffer,
    vertices: gpu::Buffer,
    indices: gpu::Buffer,
    instances: gpu::Buffer,
    /// The cubes' draw, recorded once for each count of cubes.
    bundle: Option<(u32, RenderBundle)>,
    /// The pass's beginning and end, stamped where the GPU can.
    timer: Option<Timer>,
    /// The multisampled colour and depth-stencil targets, and their size.
    targets: Option<((u32, u32), TextureView, TextureView)>,
}

struct Timer {
    set: QuerySet,
    resolved: gpu::Buffer,
    read: gpu::Buffer,
    /// A frame's stamps were copied to `read` (its work is done by the
    /// next frame: the host waits for it).
    pending: bool,
}

struct Cube {
    gpu: Option<Resources>,
    failed: Option<String>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    spin: f64,
    last_time: Option<f64>,
    paused: bool,
    reverse: bool,
    palette: usize,
    ring: usize,
    typed: u32,
    dragging: Option<(f32, f32)>,
    locked: bool,
    refresh: f32,
    frames: u32,
    counted_since: f64,
    fps: f32,
    keys: u32,
    last_key: String,
    /// The last frame's pass on the GPU, where it stamps them.
    gpu_ms: f64,
}

impl App for Cube {
    fn init(info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        let cube = Cube {
            gpu: None,
            failed: None,
            yaw: 0.6,
            pitch: 0.35,
            distance: 7.5,
            spin: 0.,
            last_time: None,
            paused: false,
            reverse: false,
            palette: 0,
            ring: 12,
            typed: 0,
            dragging: None,
            locked: false,
            refresh: info.refresh,
            frames: 0,
            counted_since: 0.,
            fps: 0.,
            keys: 0,
            last_key: String::new(),
            gpu_ms: 0.,
        };
        cube.title();
        Ok(cube)
    }

    fn frame(&mut self, time: f64) {
        let dt = self
            .last_time
            .map_or(0., |last| (time - last).clamp(0., 0.25));
        self.last_time = Some(time);
        if !self.paused {
            self.spin += dt * if self.reverse { -0.8 } else { 0.8 };
        }
        self.frames += 1;
        if time - self.counted_since >= 1. {
            self.fps = (self.frames as f64 / (time - self.counted_since)) as f32;
            self.frames = 0;
            self.counted_since = time;
            self.title();
        }
        if let Err(why) = self.draw() {
            if self.failed.as_deref() != Some(&why) {
                zgx_app::log(format!("cube: {why}"));
                host::set_title(&format!("cube failed: {why}"));
                self.failed = Some(why);
            }
        }
    }

    fn event(&mut self, event: InputEvent) {
        match event {
            InputEvent::Button(b) => match (b.button, b.down) {
                (Button::Left, true) => {
                    self.dragging = Some((b.x, b.y));
                    host::set_cursor(Cursor::Grabbing);
                }
                (Button::Left, false) => {
                    self.dragging = None;
                    host::set_cursor(Cursor::Grab);
                }
                // Granted just after a press, as the web's pointer lock.
                (Button::Right, true) => host::lock_pointer(!self.locked),
                _ => {}
            },
            InputEvent::PointerMove(p) => {
                if let Some((x, y)) = self.dragging {
                    self.turn(p.x - x, p.y - y);
                    self.dragging = Some((p.x, p.y));
                }
            }
            InputEvent::PointerMotion(m) => {
                if self.locked {
                    self.turn(m.dx * 2., m.dy * 2.);
                }
            }
            InputEvent::PointerLock(on) => {
                self.locked = on;
                host::set_cursor(if on { Cursor::Hidden } else { Cursor::Grab });
                self.title();
            }
            InputEvent::Wheel(w) => self.zoom(w.dy * 0.01),
            InputEvent::Key(key) if key.down => {
                self.keys += 1;
                self.last_key = key.code.clone();
                match key.code.as_str() {
                    "Space" => self.paused = !self.paused,
                    "KeyR" => self.reverse = !self.reverse,
                    "KeyC" => self.palette = (self.palette + 1) % PALETTES.len(),
                    "ArrowUp" => self.zoom(-0.5),
                    "ArrowDown" => self.zoom(0.5),
                    "Digit1" => self.ring = 6,
                    "Digit2" => self.ring = 12,
                    "Digit3" => self.ring = 18,
                    "Digit4" => self.ring = 24,
                    "Backspace" => self.typed = self.typed.saturating_sub(1),
                    _ => {}
                }
                self.title();
            }
            InputEvent::Text(text) => {
                self.typed += text.chars().count() as u32;
                self.title();
            }
            InputEvent::Refresh(hz) => {
                self.refresh = hz;
                self.title();
            }
            InputEvent::Focus(false) => self.dragging = None,
            // The host's device changed: what was made on the old one is
            // dead, made again in the next frame.
            InputEvent::GpuReset => self.gpu = None,
            InputEvent::PipelineReady(id) => {
                if let Some(res) = &mut self.gpu {
                    res.take_reflect(id);
                }
            }
            _ => {}
        }
    }
}

impl Cube {
    fn title(&self) {
        host::set_title(&format!(
            "cube · typed {} · keys {}{} · {:.0} Hz · {:.0} fps{}{}",
            self.typed,
            self.keys,
            if self.last_key.is_empty() {
                String::new()
            } else {
                format!(" ({})", self.last_key)
            },
            self.refresh,
            self.fps,
            if self.gpu_ms > 0. {
                format!(" · gpu {:.2} ms", self.gpu_ms)
            } else {
                String::new()
            },
            if self.locked { " · looking" } else { "" }
        ));
    }
    fn turn(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-0.2, 1.45);
    }
    fn zoom(&mut self, by: f32) {
        self.distance = (self.distance + by).clamp(3., 18.);
    }

    fn draw(&mut self) -> Result<(), String> {
        if self.gpu.is_none() {
            self.gpu = Some(Resources::new()?);
        }
        let target = gpu::current_texture()?;
        let (width, height) = (target.width(), target.height());
        let instances = self.instances();
        let count = (instances.len() / 20) as u32;
        let res = self.gpu.as_mut().expect("made above");
        if let Some(ns) = res.gpu_time()? {
            self.gpu_ms = ns / 1e6;
        }
        res.record_cubes(count)?;
        if res
            .targets
            .as_ref()
            .is_none_or(|(size, _, _)| *size != (width, height))
        {
            let usage = TextureUsage::RENDER_ATTACHMENT;
            let color = gpu::create_texture(&util::texture(
                "cube msaa",
                width,
                height,
                TextureFormat::Bgra8unorm,
                usage,
                SAMPLES,
            ))?;
            let depth = gpu::create_texture(&util::texture(
                "cube depth",
                width,
                height,
                DEPTH,
                usage,
                SAMPLES,
            ))?;
            res.targets = Some(((width, height), util::view(&color), util::view(&depth)));
        }
        let (_, msaa, depth) = res.targets.as_ref().expect("made above");

        let aspect = width as f32 / height.max(1) as f32;
        let target_at = [0., 0.4, 0.];
        let eye = [
            target_at[0] + self.distance * self.pitch.cos() * self.yaw.sin(),
            target_at[1] + self.distance * self.pitch.sin(),
            target_at[2] + self.distance * self.pitch.cos() * self.yaw.cos(),
        ];
        // The narrower side sees as much: a tall element sees the scene too.
        let fov_y = 2. * ((0.75f32 / 2.).tan() / aspect.min(1.)).atan();
        let view_proj = mul(
            &perspective(fov_y, aspect, 0.1, 100.),
            &look_at(eye, target_at, [0., 1., 0.]),
        );
        let mut uniforms = Vec::with_capacity(28);
        uniforms.extend_from_slice(&view_proj);
        uniforms.extend_from_slice(&[eye[0], eye[1], eye[2], 1.]);
        uniforms.extend_from_slice(&[-0.5, -1.0, -0.35, 0.]);
        uniforms.extend_from_slice(&[self.spin as f32, 0., 0., 0.]);
        gpu::write_buffer(&res.uniforms, 0, &bytes(&uniforms))?;
        // The same scene mirrored in the floor's top (y = -0.86), dimmed.
        let floor = -0.86;
        let mirror = [
            1.,
            0.,
            0.,
            0., //
            0.,
            -1.,
            0.,
            0., //
            0.,
            0.,
            1.,
            0., //
            0.,
            2. * floor,
            0.,
            1.,
        ];
        uniforms[..16].copy_from_slice(&mul(&view_proj, &mirror));
        uniforms[25] = 1.;
        gpu::write_buffer(&res.mirrored, 0, &bytes(&uniforms))?;
        gpu::write_buffer(&res.instances, 0, &bytes(&instances))?;
        let cubes = &res.bundle.as_ref().expect("recorded above").1;

        let frame = target.create_view(None)?;
        let encoder = gpu::create_command_encoder(Some("cube"))?;
        {
            let pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("cube".into()),
                color_attachments: vec![Some(RenderPassColorAttachment {
                    view: msaa,
                    resolve_target: Some(&frame),
                    clear: Some(Color {
                        r: 0.012,
                        g: 0.014,
                        b: 0.022,
                        a: 1.,
                    }),
                    store: StoreOp::Discard,
                })],
                depth_stencil_attachment: Some(RenderPassDepthAttachment {
                    view: depth,
                    depth_clear: Some(1.),
                    depth_store: StoreOp::Discard,
                    depth_read_only: false,
                    stencil_clear: Some(0),
                    stencil_store: Some(StoreOp::Discard),
                }),
                timestamp_writes: res.timer.as_ref().map(|t| PassTimestampWrites {
                    query_set: &t.set,
                    beginning_index: Some(0),
                    end_index: Some(1),
                }),
                occlusion_query_set: None,
            })?;
            pass.set_vertex_buffer(0, &res.vertices, 0, None);
            pass.set_vertex_buffer(1, &res.instances, 0, Some(u64::from(count) * 80));
            pass.set_index_buffer(&res.indices, IndexFormat::Uint16, 0, None);
            pass.set_stencil_reference(1);
            // The floor marked in the stencil, then the cubes mirrored only
            // there.
            pass.set_bind_group(0, Some(&res.bind_group), &[]);
            pass.set_pipeline(&res.mask);
            pass.draw_indexed(36, 1, 0, 0, 0);
            if let Some(reflect) = &res.reflect {
                pass.set_bind_group(0, Some(&res.mirrored_group), &[]);
                pass.set_pipeline(reflect);
                pass.draw_indexed(36, count - 1, 0, 0, 1);
            }
            // The floor over them, then the cubes, from their bundle.
            pass.set_bind_group(0, Some(&res.bind_group), &[]);
            pass.set_pipeline(&res.floor);
            pass.draw_indexed(36, 1, 0, 0, 0);
            pass.execute_bundles(&[cubes]);
            pass.end();
        }
        if let Some(timer) = &mut res.timer {
            encoder.resolve_query_set(&timer.set, 0, 2, &timer.resolved, 0);
            encoder.copy_buffer_to_buffer(&timer.resolved, 0, &timer.read, 0, 16);
            timer.pending = true;
        }
        let commands = CommandEncoder::finish(encoder)?;
        gpu::submit(vec![commands])
    }

    /// Each cube's model matrix and colour (alpha: how shiny).
    fn instances(&self) -> Vec<f32> {
        let palette = PALETTES[self.palette];
        let spin = self.spin as f32;
        let mut out = Vec::with_capacity(MAX_INSTANCES * 20);
        let mut push = |m: [f32; 16], c: [f32; 3], shine: f32| {
            out.extend_from_slice(&m);
            out.extend_from_slice(&[c[0], c[1], c[2], shine]);
        };
        // The floor, first: drawn apart from the cubes (instance 0).
        push(
            model([0., -0.9, 0.], 0., 0., [14., 0.08, 14.]),
            [0.16, 0.16, 0.18],
            0.1,
        );
        // The cube in the middle, bobbing.
        push(
            model(
                [0., 0.6 + (spin * 1.7).sin() * 0.15, 0.],
                spin,
                spin * 0.6,
                [1.1; 3],
            ),
            palette[0],
            1.,
        );
        // The ring, each turning its own way.
        for i in 0..self.ring {
            let a = i as f32 / self.ring as f32 * std::f32::consts::TAU - spin * 0.5;
            let r = 2.9;
            let at = [
                a.cos() * r,
                0.35 + (a * 3. + spin * 2.).sin() * 0.25,
                a.sin() * r,
            ];
            let color = if i % 2 == 0 { palette[1] } else { palette[2] };
            push(
                model(at, spin * 2. + i as f32, spin + i as f32 * 0.3, [0.42; 3]),
                color,
                1.,
            );
        }
        // A wall of what was typed, eight a row.
        for i in 0..self.typed.min(64) {
            let (col, row) = ((i % 8) as f32, (i / 8) as f32);
            push(
                model(
                    [-4.4, -0.65 + row * 0.42, -1.5 + col * 0.42],
                    0.,
                    0.,
                    [0.36; 3],
                ),
                [0.95, 0.85 - row * 0.08, 0.35 + col * 0.06],
                0.8,
            );
        }
        out
    }
}

impl Resources {
    fn new() -> Result<Self, String> {
        let module = gpu::create_shader_module(Some("cube"), SHADER)?;
        let layout = gpu::create_bind_group_layout(
            Some("cube"),
            &[util::uniform_entry(
                0,
                ShaderStage::VERTEX | ShaderStage::FRAGMENT,
            )],
        )?;
        let pipeline_layout = gpu::create_pipeline_layout(Some("cube"), &[&layout])?;
        let float4 = |location: u32, offset: u64| VertexAttribute {
            format: VertexFormat::Float32x4,
            offset,
            shader_location: location,
        };
        let buffers = vec![
            VertexBufferLayout {
                array_stride: 24,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![
                    VertexAttribute {
                        format: VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    VertexAttribute {
                        format: VertexFormat::Float32x3,
                        offset: 12,
                        shader_location: 1,
                    },
                ],
            },
            VertexBufferLayout {
                array_stride: 80,
                step_mode: VertexStepMode::Instance,
                attributes: vec![
                    float4(2, 0),
                    float4(3, 16),
                    float4(4, 32),
                    float4(5, 48),
                    float4(6, 64),
                ],
            },
        ];
        // One shader, four ways to draw with it.
        let describe = |cull: CullMode,
                        depth: DepthStencilState,
                        target: ColorTargetState|
         -> RenderPipelineDescriptor<'_> {
            RenderPipelineDescriptor {
                label: Some("cube".into()),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: Some("vs".into()),
                    buffers: buffers.clone(),
                    constants: Vec::new(),
                },
                primitive: util::primitive(PrimitiveTopology::TriangleList, cull),
                depth_stencil: Some(depth),
                multisample: util::multisample(SAMPLES),
                fragment: Some(FragmentState {
                    module: &module,
                    entry_point: Some("fs".into()),
                    targets: vec![Some(target)],
                    constants: Vec::new(),
                }),
            }
        };
        let depth = util::depth(DEPTH, CompareFunction::Less);
        let opaque = util::target(TextureFormat::Bgra8unorm);
        let pipeline = gpu::create_render_pipeline(&describe(CullMode::Back, depth, opaque))?;
        let over = BlendComponent {
            operation: BlendOperation::Add,
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
        };
        let floor = gpu::create_render_pipeline(&describe(
            CullMode::Back,
            depth,
            ColorTargetState {
                blend: Some(BlendState {
                    color: over,
                    alpha: over,
                }),
                ..opaque
            },
        ))?;
        let mask = gpu::create_render_pipeline(&describe(
            CullMode::Back,
            DepthStencilState {
                depth_write_enabled: false,
                stencil: Some(util::stencil(
                    CompareFunction::Always,
                    StencilOperation::Replace,
                )),
                ..depth
            },
            ColorTargetState {
                write_mask: ColorWrite::empty(),
                ..opaque
            },
        ))?;
        // Mirrored, its faces turn the other way.
        let reflect_future = gpu::create_render_pipeline_async(&describe(
            CullMode::Front,
            DepthStencilState {
                stencil: Some(util::stencil(
                    CompareFunction::Equal,
                    StencilOperation::Keep,
                )),
                ..depth
            },
            opaque,
        ))?;
        let uniform = |label: &str| {
            gpu::create_buffer(&BufferDescriptor {
                label: Some(label.into()),
                size: 112,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            })
        };
        let group = |buffer: &gpu::Buffer| {
            gpu::create_bind_group(
                Some("cube"),
                &layout,
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            )
        };
        let (uniforms, mirrored) = (uniform("cube scene")?, uniform("cube mirrored")?);
        let (bind_group, mirrored_group) = (group(&uniforms)?, group(&mirrored)?);
        let (vertices, indices) = box_mesh();
        let vertices = gpu::create_buffer_with_data(
            Some("cube vertices"),
            BufferUsage::VERTEX,
            &bytes(&vertices),
        )?;
        let indices = gpu::create_buffer_with_data(
            Some("cube indices"),
            BufferUsage::INDEX,
            &indices
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<u8>>(),
        )?;
        let instances = gpu::create_buffer(&BufferDescriptor {
            label: Some("cube instances".into()),
            size: (MAX_INSTANCES * 80) as u64,
            usage: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        })?;
        let timer = if gpu::supports_timestamps() {
            Some(Timer {
                set: gpu::create_query_set(&QuerySetDescriptor {
                    label: Some("cube pass".into()),
                    kind: QueryType::Timestamp,
                    count: 2,
                })?,
                resolved: gpu::create_buffer(&BufferDescriptor {
                    label: None,
                    size: 16,
                    usage: BufferUsage::QUERY_RESOLVE | BufferUsage::COPY_SRC,
                })?,
                read: gpu::create_buffer(&BufferDescriptor {
                    label: None,
                    size: 16,
                    usage: BufferUsage::COPY_DST | BufferUsage::MAP_READ,
                })?,
                pending: false,
            })
        } else {
            None
        };
        Ok(Resources {
            pipeline,
            floor,
            mask,
            reflect: None,
            reflect_future: Some(reflect_future),
            bind_group,
            mirrored_group,
            uniforms,
            mirrored,
            vertices,
            indices,
            instances,
            bundle: None,
            timer,
            targets: None,
        })
    }
    /// The reflections' pipeline, made on another thread.
    fn take_reflect(&mut self, id: u64) {
        let Some(future) = &self.reflect_future else {
            return;
        };
        if future.id() != id {
            return;
        }
        match future.take() {
            Some(Ok(pipeline)) => self.reflect = Some(pipeline),
            Some(Err(why)) => zgx_app::log(format!("cube: reflections: {why}")),
            None => return,
        }
        self.reflect_future = None;
    }
    /// The bundle drawing the cubes (instances 1 to `count`), recorded
    /// again only as their count changes.
    fn record_cubes(&mut self, count: u32) -> Result<(), String> {
        if self.bundle.as_ref().is_none_or(|(n, _)| *n != count) {
            let encoder = gpu::create_render_bundle_encoder(&RenderBundleEncoderDescriptor {
                label: Some("cubes".into()),
                color_formats: vec![Some(TextureFormat::Bgra8unorm)],
                depth_stencil: Some(RenderBundleDepthStencil {
                    format: DEPTH,
                    depth_read_only: false,
                    stencil_read_only: true,
                }),
                sample_count: SAMPLES,
            })?;
            encoder.set_pipeline(&self.pipeline);
            encoder.set_bind_group(0, Some(&self.bind_group), &[]);
            encoder.set_vertex_buffer(0, &self.vertices, 0, None);
            encoder.set_vertex_buffer(1, &self.instances, 0, None);
            encoder.set_index_buffer(&self.indices, IndexFormat::Uint16, 0, None);
            encoder.draw_indexed(36, count - 1, 0, 0, 1);
            self.bundle = Some((count, RenderBundleEncoder::finish(encoder, Some("cubes"))?));
        }
        Ok(())
    }
    /// The last frame's pass, in nanoseconds, where the GPU stamps them.
    fn gpu_time(&mut self) -> Result<Option<f64>, String> {
        let Some(timer) = &mut self.timer else {
            return Ok(None);
        };
        if !std::mem::take(&mut timer.pending) {
            return Ok(None);
        }
        let stamps = gpu::read_buffer(&timer.read, 0, 16)?;
        let begin = u64::from_le_bytes(stamps[..8].try_into().unwrap());
        let end = u64::from_le_bytes(stamps[8..].try_into().unwrap());
        Ok((end > begin).then(|| (end - begin) as f64 * f64::from(gpu::get_timestamp_period())))
    }
}

/// A unit box: four vertices a face (position, normal), wound outward.
fn box_mesh() -> (Vec<f32>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
        ([-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]),
        ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.]),
        ([0., -1., 0.], [1., 0., 0.], [0., 0., 1.]),
        ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.]),
        ([0., 0., -1.], [0., 1., 0.], [1., 0., 0.]),
    ];
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for (n, u, v) in faces {
        let base = (vertices.len() / 6) as u16;
        for (su, sv) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
            for k in 0..3 {
                vertices.push((n[k] + su * u[k] + sv * v[k]) * 0.5);
            }
            vertices.extend_from_slice(&n);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Column-major 4×4 matrices, WebGPU's clip space (depth 0 to 1).
fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    out
}
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1. / (fov_y / 2.).tan();
    [
        f / aspect,
        0.,
        0.,
        0.,
        0.,
        f,
        0.,
        0.,
        0.,
        0.,
        far / (near - far),
        -1.,
        0.,
        0.,
        near * far / (near - far),
        0.,
    ]
}
fn look_at(eye: [f32; 3], at: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let z = normalize(sub(eye, at));
    let x = normalize(cross(up, z));
    let y = cross(z, x);
    [
        x[0],
        y[0],
        z[0],
        0.,
        x[1],
        y[1],
        z[1],
        0.,
        x[2],
        y[2],
        z[2],
        0.,
        -dot(x, eye),
        -dot(y, eye),
        -dot(z, eye),
        1.,
    ]
}
/// Translated, turned about y then x, scaled.
fn model(at: [f32; 3], yaw: f32, pitch: f32, scale: [f32; 3]) -> [f32; 16] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    // R = Ry * Rx, columns.
    let x = [cy, 0., -sy];
    let y = [sy * sp, cp, cy * sp];
    let z = [sy * cp, -sp, cy * cp];
    [
        x[0] * scale[0],
        x[1] * scale[0],
        x[2] * scale[0],
        0.,
        y[0] * scale[1],
        y[1] * scale[1],
        y[2] * scale[1],
        0.,
        z[0] * scale[2],
        z[1] * scale[2],
        z[2] * scale[2],
        0.,
        at[0],
        at[1],
        at[2],
        1.,
    ]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

zgx_app::app!(Cube);
