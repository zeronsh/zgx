//! A painter's side of its world's particles: each emitter's pool on the
//! device, the update pass recorded before the frame, and the draw after
//! the meshes.
//!
//! Until the renderer's pass list draws particles inside its main pass
//! (with its depth), they're drawn in a pass of their own over the frame:
//! in front of every mesh.
use super::{
    Blend, CPU_PARTICLES, Collide, Force, Live, MAX_STEP, Particles, Region, Response, Shape,
    UNSETTLED,
    cpu::{self, Particle},
};
use crate::world::Inner;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Weak,
    sync::{Arc, Mutex},
};
use zgx_gpu::{viewport::ViewportFrame, wgpu};

/// What the update shader reads (`Emitter` in `update.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniforms {
    pub(crate) spawn: [u32; 4],
    pub(crate) a: [f32; 4],
    pub(crate) b: [f32; 4],
    pub(crate) velocity: [f32; 4],
    pub(crate) life: [f32; 4],
    pub(crate) accel: [f32; 4],
    pub(crate) wind: [f32; 4],
    pub(crate) swirl: [f32; 4],
    pub(crate) collide: [f32; 4],
    pub(crate) color: [f32; 4],
    /// Its sub-emitter's slots, and births a death and a hit give it.
    pub(crate) events: [u32; 4],
}

/// What the draw shader reads (`Draw` in `draw.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniforms {
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    look: [f32; 4],
    shape: [f32; 4],
    colors: [[f32; 4]; 4],
    sizes: [f32; 4],
}

/// `live`'s numbers for a frame spawning `count` from slot `base`, at
/// `time`, `step` seconds on, in a pool of `slots`.
fn uniforms(
    live: &Live,
    (base, count, slots): (u32, u32, u32),
    time: f64,
    step: f32,
    seed: u32,
) -> Uniforms {
    let e = &live.emitter;
    let o = live.offset;
    let moved = |p: [f32; 3]| [p[0] + o[0], p[1] + o[1], p[2] + o[2]];
    let (kind, a, b) = match e.spawn.region {
        Region::Point(p) => (0, moved(p).into_iter().chain([0.]).collect(), [0.; 4]),
        Region::Box(min, max) => {
            let (min, max) = (moved(min), moved(max));
            (
                1,
                vec![min[0], min[1], min[2], 0.],
                [max[0], max[1], max[2], 0.],
            )
        }
        Region::Sphere(c, r) => (2, moved(c).into_iter().chain([r]).collect(), [0.; 4]),
        Region::Cone(apex, axis, angle) => (
            3,
            moved(apex).into_iter().chain([angle]).collect(),
            [axis[0], axis[1], axis[2], 0.],
        ),
    };
    let mut accel = [0f32; 4];
    let mut wind = [0f32; 4];
    let mut swirl = [0f32; 4];
    for force in &e.forces {
        match *force {
            Force::Gravity => accel[1] -= 9.81,
            Force::Constant(c) => (0..3).for_each(|i| accel[i] += c[i]),
            Force::Drag(k) => accel[3] += k.max(0.),
            Force::Wind(w) => wind[..3].copy_from_slice(&w),
            Force::Swirl { strength, scale } => {
                wind[3] += strength;
                swirl[0] = scale;
            }
        }
    }
    swirl[1] = (time % 3600.) as f32;
    let mut collide = [0., 0., e.size.start, e.size.end];
    if let Collide::Plane { height, then } = e.collide {
        swirl[2] = height;
        match then {
            Response::Bounce { keep, friction } => {
                collide[0] = 1.;
                collide[1] = friction.clamp(0., 1.);
                swirl[3] = keep.clamp(0., 1.);
            }
            Response::Stick => collide[0] = 2.,
            Response::Die => collide[0] = 3.,
        }
    }
    Uniforms {
        spawn: [kind, base, count, slots],
        a: a.try_into().unwrap(),
        b,
        velocity: [
            e.velocity.0[0],
            e.velocity.0[1],
            e.velocity.0[2],
            e.velocity.1,
        ],
        life: [e.life.start, e.life.end, step, f32::from_bits(seed)],
        accel,
        wind,
        swirl,
        collide,
        color: e.color,
        events: [0; 4],
    }
}

/// An emitter's pool on the device.
struct Pool {
    slots: u32,
    particles: wgpu::Buffer,
    update: wgpu::Buffer,
    draw: wgpu::Buffer,
    update_bind: Option<wgpu::BindGroup>,
    draw_bind: wgpu::BindGroup,
    /// Its particles, where the CPU updates them.
    cpu: Option<Vec<Particle>>,
    /// Particles its parents gave birth to (the next goes in this, modulo
    /// its slots): on the device, or the CPU's count.
    born: Option<wgpu::Buffer>,
    born_cpu: u32,
    /// This frame's numbers, for parents giving birth into it.
    last: Uniforms,
    /// The device refused to make it: neither updated nor drawn.
    failed: bool,
    /// Whether any may live this frame (else neither updated nor drawn).
    shown: bool,
    blend: Blend,
    /// The frame it was last stepped in.
    used: u64,
}

/// Pipelines made on a thread of their own with the first frame.
struct Pipelines {
    update: Option<wgpu::ComputePipeline>,
    /// `update` giving birth into a sub-emitter's pool.
    update_events: Option<wgpu::ComputePipeline>,
    /// Alpha's and additive's, for the frame's format.
    draw: [wgpu::RenderPipeline; 2],
}
type Compiling = Arc<Mutex<Option<Pipelines>>>;

#[derive(Default)]
pub(crate) struct Device {
    made: bool,
    compute: bool,
    update_layout: Option<wgpu::BindGroupLayout>,
    events_layout: Option<wgpu::BindGroupLayout>,
    draw_layout: Option<wgpu::BindGroupLayout>,
    format: Option<wgpu::TextureFormat>,
    compiling: Option<Compiling>,
    pipelines: Option<Pipelines>,
    pools: HashMap<u64, Pool>,
    last: Option<std::time::Instant>,
    index: u64,
}

impl Device {
    pub(crate) fn device_changed(&mut self) {
        *self = Self {
            last: self.last,
            index: self.index,
            ..Self::default()
        };
    }

    /// Before the frame: each emitter's step recorded (or taken on the
    /// CPU). Whether the world has emitters (`after` is called only if so).
    #[inline]
    pub(crate) fn before(
        &mut self,
        world: &Weak<RefCell<Inner>>,
        frame: &mut ViewportFrame<'_>,
    ) -> bool {
        let Some(world) = world.upgrade() else {
            return false;
        };
        let mut inner = world.borrow_mut();
        let Some(mut particles) = inner.particles.take() else {
            return false;
        };
        self.step(&mut particles, &inner, frame);
        inner.particles = Some(particles);
        true
    }

    #[cold]
    #[inline(never)]
    fn step(&mut self, particles: &mut Particles, inner: &Inner, frame: &mut ViewportFrame<'_>) {
        let device = frame.device;
        let step = self.last.map_or(0., |last| {
            frame
                .time
                .saturating_duration_since(last)
                .as_secs_f32()
                .min(MAX_STEP)
        });
        self.last = Some(frame.time);
        self.index += 1;
        let format = frame
            .target_srgb
            .map_or(frame.format, |_| frame.format.add_srgb_suffix());
        if !self.made {
            self.made = true;
            self.compute = crate::Caps::of(device).compute;
        }
        if self.format != Some(format) {
            self.format = Some(format);
            self.pipelines = None;
            self.compiling = Some(self.compile(device, format));
        }
        if let Some(compiling) = &self.compiling {
            let done = compiling.lock().unwrap_or_else(|p| p.into_inner()).take();
            if let Some(done) = done {
                self.pipelines = Some(done);
                self.compiling = None;
            }
        }
        let Some(pipelines) = &self.pipelines else {
            return;
        };
        // Frames step it once it's drawn: its clock starts then.
        particles.ready = true;
        particles.time += f64::from(step);
        particles.steps = particles.steps.wrapping_add(1);
        let time = particles.time;
        let on_gpu = self.compute && !particles.on_cpu && pipelines.update.is_some();
        let (Some(update_layout), Some(draw_layout)) = (&self.update_layout, &self.draw_layout)
        else {
            return;
        };
        let aspect = frame.size.0 as f32 / frame.size.1.max(1) as f32;
        let look = [
            match inner.tone_mapping {
                crate::ToneMapping::Neutral => 0.,
                crate::ToneMapping::Aces => 1.,
                crate::ToneMapping::Linear => 2.,
            },
            inner.exposure.exp2(),
            if frame.target_srgb.is_none() && !frame.format.is_srgb() {
                1.
            } else {
                0.
            },
            2. / frame.size.1.max(1) as f32,
        ];
        let (view, projection) = (inner.camera.view(), inner.camera.projection(aspect));
        let index = self.index;
        let seed_base = particles.steps.wrapping_mul(0x9e37_79b9);
        let slots_of = |live: &Live| {
            if on_gpu {
                live.emitter.max
            } else {
                live.emitter.max.min(CPU_PARTICLES)
            }
        };
        particles.feed();
        particles.pools.clear();
        let mut pass: Option<wgpu::ComputePass<'static>> = None;
        let mut events: Vec<(u32, [f32; 3], u32)> = Vec::new();
        // Sub-emitters before their parents: a parent's births land in
        // their pools after their own step.
        for at in particles.order() {
            let child = particles.emitters[at]
                .1
                .as_ref()
                .and_then(|live| live.emitter.sub)
                .and_then(|(child, death, hit)| {
                    let live = particles.child(child)?;
                    Some((live.id, slots_of(live), death, hit))
                })
                .filter(|&(id, ..)| self.pools.contains_key(&id));
            let Some(live) = particles.emitters[at].1.as_mut() else {
                continue;
            };
            let slots = slots_of(live);
            let pool = self
                .pools
                .entry(live.id)
                .or_insert_with(|| make_pool(device, (update_layout, draw_layout), slots, on_gpu));
            // Made for the other way of updating: made again, from this frame.
            if pool.cpu.is_some() == on_gpu || pool.slots != slots {
                *pool = make_pool(device, (update_layout, draw_layout), slots, on_gpu);
            }
            particles.pools.push((pool.slots, pool.cpu.is_some()));
            pool.used = index;
            pool.blend = live.emitter.blend;
            if pool.failed {
                pool.shown = false;
                continue;
            }
            let count = live.step(time, step, slots);
            let base = live.head;
            live.head = (live.head + count) % slots;
            pool.shown = live.unsettled(time) || count > 0;
            if !pool.shown {
                continue;
            }
            let mut u = uniforms(
                live,
                (base, count, slots),
                time,
                step,
                seed_base ^ live.id as u32,
            );
            if let Some((_, child_slots, death, hit)) = child {
                u.events = [child_slots, death, hit, 0];
            }
            pool.last = u;
            events.clear();
            match &mut pool.cpu {
                Some(cpu) => {
                    if cpu::step(cpu, &u, &mut events) {
                        frame
                            .queue
                            .write_buffer(&pool.particles, 0, bytemuck::cast_slice(cpu));
                    }
                }
                None => frame
                    .queue
                    .write_buffer(&pool.update, 0, bytemuck::bytes_of(&u)),
            }
            let e = &live.emitter;
            let draw = DrawUniforms {
                view: view.columns,
                projection: projection.columns,
                look,
                shape: match e.shape {
                    Shape::Billboard => [0.; 4],
                    Shape::Streak { length } => [1., length.max(0.), 0., 0.],
                },
                colors: e.colors,
                sizes: e.sizes,
            };
            frame
                .queue
                .write_buffer(&pool.draw, 0, bytemuck::bytes_of(&draw));
            let id = live.id;
            let pool = &self.pools[&id];
            if pool.cpu.is_none() {
                let compute = pass.get_or_insert_with(|| {
                    frame
                        .encoder
                        .begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("3D particles"),
                            timestamp_writes: None,
                        })
                        .forget_lifetime()
                });
                let feeding = child
                    .zip(pipelines.update_events.as_ref())
                    .zip(self.events_layout.as_ref());
                match feeding {
                    Some(((child, pipeline), layout)) => {
                        let into = &self.pools[&child.0];
                        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("3D particles giving birth"),
                            layout,
                            entries: &[
                                entry(0, &pool.update),
                                entry(1, &pool.particles),
                                entry(2, &into.update),
                                entry(3, &into.particles),
                                entry(4, into.born.as_ref().unwrap()),
                            ],
                        });
                        compute.set_pipeline(pipeline);
                        compute.set_bind_group(0, &bind, &[]);
                    }
                    None => {
                        compute.set_pipeline(pipelines.update.as_ref().unwrap());
                        compute.set_bind_group(0, pool.update_bind.as_ref().unwrap(), &[]);
                    }
                }
                compute.dispatch_workgroups(slots.div_ceil(256), 1, 1);
            } else if let Some((child, ..)) = child
                && !events.is_empty()
                && let Some(into) = self.pools.get_mut(&child)
                && let Some(children) = &mut into.cpu
            {
                cpu::give_birth(children, &mut into.born_cpu, &into.last, &events);
                frame
                    .queue
                    .write_buffer(&into.particles, 0, bytemuck::cast_slice(children));
            }
        }
        drop(pass);
        // Pools of emitters removed, or made for another way of updating.
        self.pools
            .retain(|_, pool| pool.used == index && pool.cpu.is_none() == on_gpu);
    }

    /// After the meshes: the particles drawn over the frame, and whether
    /// any keeps the world drawing.
    #[cold]
    #[inline(never)]
    pub(crate) fn after(&mut self, world: &Weak<RefCell<Inner>>, frame: &mut ViewportFrame<'_>) {
        let Some(world) = world.upgrade() else {
            return;
        };
        if let Some(pipelines) = &self.pipelines
            && self.pools.values().any(|pool| pool.shown)
        {
            let target = frame.target_srgb.unwrap_or(frame.target);
            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("3D particles"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            // Additive after alpha: light added over what alpha covers.
            for blend in [Blend::Alpha, Blend::Additive] {
                for pool in self.pools.values().filter(|p| p.shown && p.blend == blend) {
                    pass.set_pipeline(&pipelines.draw[blend as usize]);
                    pass.set_bind_group(0, &pool.draw_bind, &[]);
                    pass.set_vertex_buffer(0, pool.particles.slice(..));
                    pass.draw(0..4, 0..pool.slots);
                }
            }
        }
        let inner = world.borrow();
        let unsettled = inner.particles.as_ref().is_some_and(|p| p.unsettled());
        inner.unsettle(UNSETTLED, unsettled);
    }

    /// The update and draw pipelines for `format`, made on a thread of
    /// their own.
    fn compile(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> Compiling {
        let update_layout = self
            .update_layout
            .get_or_insert_with(|| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("3D particles update"),
                    entries: &[
                        layout_entry(
                            0,
                            wgpu::ShaderStages::COMPUTE,
                            wgpu::BufferBindingType::Uniform,
                        ),
                        layout_entry(
                            1,
                            wgpu::ShaderStages::COMPUTE,
                            wgpu::BufferBindingType::Storage { read_only: false },
                        ),
                    ],
                })
            })
            .clone();
        let events_layout = self
            .events_layout
            .get_or_insert_with(|| {
                let storage = wgpu::BufferBindingType::Storage { read_only: false };
                let uniform = wgpu::BufferBindingType::Uniform;
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("3D particles giving birth"),
                    entries: &[
                        layout_entry(0, wgpu::ShaderStages::COMPUTE, uniform),
                        layout_entry(1, wgpu::ShaderStages::COMPUTE, storage),
                        layout_entry(2, wgpu::ShaderStages::COMPUTE, uniform),
                        layout_entry(3, wgpu::ShaderStages::COMPUTE, storage),
                        layout_entry(4, wgpu::ShaderStages::COMPUTE, storage),
                    ],
                })
            })
            .clone();
        let draw_layout = self
            .draw_layout
            .get_or_insert_with(|| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("3D particles draw"),
                    entries: &[layout_entry(
                        0,
                        wgpu::ShaderStages::VERTEX_FRAGMENT,
                        wgpu::BufferBindingType::Uniform,
                    )],
                })
            })
            .clone();
        let slot: Compiling = Arc::default();
        let done = slot.clone();
        let (device, compute) = (device.clone(), self.compute);
        let make = move || {
            let module = compute.then(|| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("3D particles update"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("update.wgsl").into()),
                })
            });
            let update_pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
                let module = module.as_ref()?;
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("3D particles update"),
                    bind_group_layouts: &[Some(layout)],
                    immediate_size: 0,
                });
                Some(
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("3D particles update"),
                        layout: Some(&layout),
                        module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                )
            };
            let update = update_pipeline(&update_layout, "update");
            let update_events = update_pipeline(&events_layout, "update_events");
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("3D particles draw"),
                source: wgpu::ShaderSource::Wgsl(include_str!("draw.wgsl").into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D particles draw"),
                bind_group_layouts: &[Some(&draw_layout)],
                immediate_size: 0,
            });
            let draw = [Blend::Alpha, Blend::Additive]
                .map(|blend| draw_pipeline(&device, &layout, &module, format, blend));
            *done.lock().unwrap_or_else(|p| p.into_inner()) = Some(Pipelines {
                update,
                update_events,
                draw,
            });
        };
        if let Err(error) = crate::compute::compile_off_the_frame(make) {
            eprintln!("zgx-3d: particles' pipelines can't be made ({error}): none drawn");
        }
        slot
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn draw_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    blend: Blend,
) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32, 4 => Float32x4, 5 => Float32
    ];
    let component = match blend {
        Blend::Alpha => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        Blend::Additive => wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("3D particles draw"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Particle>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &ATTRIBUTES,
            }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: component,
                    alpha: component,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn make_pool(
    device: &wgpu::Device,
    (update_layout, draw_layout): (&wgpu::BindGroupLayout, &wgpu::BindGroupLayout),
    slots: u32,
    on_gpu: bool,
) -> Pool {
    // Checked against the device's limits when the emitter was added; a
    // refusal still left here, not to the device's handler.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pool = make_pool_in_scope(device, (update_layout, draw_layout), slots, on_gpu);
    if let Some(error) = crate::compute::now(scope.pop()) {
        eprintln!("zgx-3d: a particle pool of {slots} couldn't be made: {error}");
        pool.failed = true;
    }
    pool
}

fn make_pool_in_scope(
    device: &wgpu::Device,
    (update_layout, draw_layout): (&wgpu::BindGroupLayout, &wgpu::BindGroupLayout),
    slots: u32,
    on_gpu: bool,
) -> Pool {
    let particles = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("3D particles"),
        size: u64::from(slots) * std::mem::size_of::<Particle>() as u64,
        usage: wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::COPY_DST
            | if on_gpu {
                wgpu::BufferUsages::STORAGE
            } else {
                wgpu::BufferUsages::empty()
            },
        mapped_at_creation: false,
    });
    let uniform = |label, size| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let update = uniform(
        "3D particles update",
        std::mem::size_of::<Uniforms>() as u64,
    );
    let draw = uniform(
        "3D particles draw",
        std::mem::size_of::<DrawUniforms>() as u64,
    );
    let update_bind = on_gpu.then(|| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D particles update"),
            layout: update_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: update.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particles.as_entire_binding(),
                },
            ],
        })
    });
    let draw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3D particles draw"),
        layout: draw_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: draw.as_entire_binding(),
        }],
    });
    Pool {
        slots,
        particles,
        update,
        draw,
        update_bind,
        draw_bind,
        cpu: (!on_gpu).then(|| vec![Particle::default(); slots as usize]),
        born: on_gpu.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D particles born"),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        }),
        born_cpu: 0,
        last: Uniforms::default(),
        failed: false,
        shown: false,
        blend: Blend::Alpha,
        used: 0,
    }
}
