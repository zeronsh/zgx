//! An external surface's client on wgpu 29: a ring of dots turning, drawn
//! into the host's buffers, reacting to the input the host forwards.
//!
//! Started by a zgx host (`zgx_surface::host::App`, Luau's `ui.app`), never
//! alone: it finds its connection in `ZGX_SURFACE_FD`.
//!
//! - drag to turn the ring; click to lock the pointer, then the mouse turns
//!   it (Escape lets go);
//! - keys: C changes the colour, Space stops and starts the spin, the
//!   wheel zooms;
//! - `--shm`: offer shared memory only (drawn on any adapter, read back and
//!   copied), to compare with shared Vulkan images or IOSurfaces;
//! - `--fps N`: draw at most N frames a second (default: as the host shows
//!   them).
//!
//! Every two seconds it prints `SPINNER fps=… kind=…` on stderr.
use std::time::{Duration, Instant};
use zgx_surface::{
    BufferDesc, BufferKind, Button, Client, Cursor, HostMessage, Offer,
    shm::Mapping,
    wgpu29::{self, Readback, wgpu},
};

struct Target {
    /// What the frame is drawn into: the shared textures (Vulkan) or one
    /// texture of its own (shm), read back into `maps`.
    textures: Vec<wgpu::Texture>,
    maps: Vec<(Mapping, usize)>,
    readback: Option<Readback>,
    size: (u32, u32),
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shm_only = args.iter().any(|a| a == "--shm");
    let fps_cap = args
        .iter()
        .position(|a| a == "--fps")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<f64>().ok());

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let mut offers = Vec::new();
    if !shm_only {
        offers.extend(wgpu29::vulkan_offer(&adapters));
        offers.extend(wgpu29::iosurface_offer(&adapters));
    }
    offers.push(Offer::Shm);
    let mut client =
        Client::connect(offers).expect("spinner_client runs inside a zgx host (ZGX_SURFACE_FD)");
    let welcome = client.welcome().clone();
    let adapter = match (welcome.kind, &welcome.device) {
        (BufferKind::Vulkan | BufferKind::IoSurface, Some(ids)) => {
            wgpu29::adapter_for(adapters, ids).expect("the host's GPU")
        }
        _ => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("an adapter"),
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device");
    eprintln!(
        "SPINNER connected kind={:?} adapter={}",
        welcome.kind,
        adapter.get_info().name
    );
    let _ = client.set_title("Spinner");
    let _ = client.set_cursor(Cursor::Crosshair);

    let scene = Scene::new(&device);
    let mut target: Option<Target> = None;
    let mut visible = true;
    let mut state = State {
        pointer: (0., 0.),
        angle: 0.,
        hue: 0.58,
        zoom: 1.,
        spinning: true,
        locked: false,
        dragging: None,
    };
    let start = Instant::now();
    let mut last = Instant::now();
    let mut counted = (Instant::now(), 0u32);
    let mut last_frame = Instant::now();

    loop {
        while let Some(event) = client.poll() {
            match event {
                HostMessage::Buffers(generation) => {
                    let size = (generation.width, generation.height);
                    let mut next = Target {
                        textures: Vec::new(),
                        maps: Vec::new(),
                        readback: None,
                        size,
                    };
                    for buffer in generation.buffers {
                        match &buffer.desc {
                            BufferDesc::Vulkan(_) | BufferDesc::IoSurface { .. } => {
                                // Safety: on the adapter the welcome named.
                                let texture = unsafe { wgpu29::import(&device, buffer) }
                                    .expect("importing a buffer");
                                next.textures.push(texture);
                            }
                            BufferDesc::Shm { stride, len, .. } => {
                                let (stride, len) = (*stride as usize, *len as usize);
                                let handle = buffer.handle.expect("shared memory");
                                next.maps
                                    .push((Mapping::map(&handle, len).expect("mapping"), stride));
                            }
                            _ => {}
                        }
                    }
                    if !next.maps.is_empty() {
                        next.textures
                            .push(device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("spinner"),
                                size: wgpu::Extent3d {
                                    width: size.0,
                                    height: size.1,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu29::FORMAT,
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                    | wgpu::TextureUsages::COPY_SRC,
                                view_formats: &[],
                            }));
                        next.readback = Some(Readback::new(&device, size.0, size.1));
                    }
                    target = Some(next);
                }
                HostMessage::Visible(shown) => visible = shown,
                HostMessage::PointerMove { x, y } => {
                    if let Some((from, angle)) = state.dragging {
                        state.angle = angle + (x - from) * 0.01;
                    }
                    state.pointer = (x, y);
                }
                HostMessage::PointerMotion { dx, .. } => state.angle += dx * 0.01,
                HostMessage::Button {
                    button: Button::Left,
                    down,
                    x,
                    ..
                } => {
                    if down && !state.locked {
                        // A click locks (the press makes it allowed); a
                        // drag before the lock arrives turns the ring.
                        let _ = client.lock_pointer(true);
                        state.dragging = Some((x, state.angle));
                    } else if !down {
                        state.dragging = None;
                    }
                }
                HostMessage::PointerLock(locked) => {
                    state.locked = locked;
                    state.dragging = None;
                }
                HostMessage::Key(key) if key.down && !key.repeat => match key.code.as_str() {
                    "KeyC" => state.hue = (state.hue + 0.13) % 1.,
                    "Space" => state.spinning = !state.spinning,
                    _ => {}
                },
                HostMessage::Wheel { dy, .. } => {
                    state.zoom = (state.zoom * (1. - dy * 0.002)).clamp(0.3, 3.)
                }
                HostMessage::Close => return,
                _ => {}
            }
        }
        if client.is_closed() {
            return;
        }
        let Some(frame_target) = &target else {
            client.wait(None);
            continue;
        };
        if !visible {
            // Hidden: nothing drawn until it shows (or anything else comes).
            client.wait(None);
            continue;
        }
        // Paced by the host: draw only with two buffers free (the host then
        // holds one showing and none waiting), so no frame is drawn that
        // would be replaced before it shows.
        if client.free_buffers() < 2 {
            client.wait(None);
            continue;
        }
        let Some(index) = client.acquire() else {
            continue;
        };
        if let Some(fps) = fps_cap {
            let due = last_frame + Duration::from_secs_f64(1. / fps);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        last_frame = Instant::now();
        let now = Instant::now();
        if state.spinning {
            state.angle += now.duration_since(last).as_secs_f32() * 1.2;
        }
        last = now;
        let texture = if frame_target.maps.is_empty() {
            &frame_target.textures[index as usize]
        } else {
            &frame_target.textures[0]
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.draw(
            &queue,
            &mut encoder,
            texture,
            frame_target.size,
            &state,
            start.elapsed().as_secs_f32(),
        );
        if let Some(readback) = &frame_target.readback {
            readback.copy(&mut encoder, texture);
        }
        let submission = queue.submit([encoder.finish()]);
        // v1 shares no semaphore: the frame is finished before it's presented.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        if let Some(readback) = &frame_target.readback {
            let (map, stride) = &frame_target.maps[index as usize];
            // Safety: acquired, the buffer is this side's until presented.
            let _ = readback.write(&device, unsafe { map.bytes_mut() }, *stride);
        }
        if client.present(index).is_err() {
            return;
        }
        counted.1 += 1;
        let elapsed = counted.0.elapsed();
        if elapsed >= Duration::from_secs(2) {
            eprintln!(
                "SPINNER fps={:.1} kind={:?} size={}x{}",
                f64::from(counted.1) / elapsed.as_secs_f64(),
                welcome.kind,
                frame_target.size.0,
                frame_target.size.1
            );
            counted = (Instant::now(), 0);
        }
    }
}

struct State {
    pointer: (f32, f32),
    angle: f32,
    hue: f32,
    zoom: f32,
    spinning: bool,
    locked: bool,
    /// Where a drag began and the angle then.
    dragging: Option<(f32, f32)>,
}

struct Scene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
}
impl Scene {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spinner"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spinner"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spinner"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spinner"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spinner"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("spinner"),
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
                targets: &[Some(wgpu29::FORMAT.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms,
            bind,
        }
    }
    fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        size: (u32, u32),
        state: &State,
        time: f32,
    ) {
        let values: [f32; 12] = [
            size.0 as f32,
            size.1 as f32,
            state.pointer.0,
            state.pointer.1,
            time,
            state.angle,
            state.hue,
            state.zoom,
            if state.locked { 1. } else { 0. },
            0.,
            0.,
            0.,
        ];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&self.uniforms, 0, &bytes);
        let view = texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spinner"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

const SHADER: &str = r#"
struct U { size: vec2<f32>, pointer: vec2<f32>, time: f32, angle: f32, hue: f32, zoom: f32, locked: f32, _p0: f32, _p1: f32, _p2: f32 }
@group(0) @binding(0) var<uniform> u: U;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}
fn hsv(h: f32, s: f32, v: f32) -> vec3<f32> {
    let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0));
    return v * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), s);
}
@fragment fn fragment(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = at.xy;
    let c = u.size * 0.5;
    let unit = min(u.size.x, u.size.y);
    // A diagonal gradient in the chosen hue.
    let t = (p.x + p.y) / (u.size.x + u.size.y);
    var color = mix(hsv(u.hue, 0.6, 0.18), hsv(u.hue + 0.1, 0.7, 0.42), t);
    // Twelve dots on a ring, turning, the leading one brightest.
    let radius = unit * 0.3 * u.zoom;
    for (var i = 0; i < 12; i++) {
        let a = u.angle + f32(i) * 6.2831853 / 12.0;
        let d = c + vec2<f32>(cos(a), sin(a)) * radius;
        let r = unit * 0.035 * u.zoom;
        let fade = f32(i + 1) / 12.0;
        let k = clamp(r - length(p - d), 0.0, 1.0);
        color = mix(color, hsv(u.hue + 0.5, 0.5, 1.0) * fade, k);
    }
    // The pointer, a cross where the host last saw it.
    let q = abs(p - u.pointer);
    if (u.locked < 0.5 && ((q.x < 1.0 && q.y < 12.0) || (q.y < 1.0 && q.x < 12.0))) {
        color = vec3<f32>(1.0);
    }
    // Locked: a border.
    let edge = min(min(p.x, p.y), min(u.size.x - p.x, u.size.y - p.y));
    if (u.locked > 0.5 && edge < 6.0) {
        color = vec3<f32>(1.0, 0.8, 0.2);
    }
    return vec4<f32>(color, 1.0);
}
"#;
