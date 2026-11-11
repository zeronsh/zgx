//! A painter's side of its world's particles: each emitter's pool on the
//! device and the update pass recorded before the frame; the pools then
//! handed to the world's drawer (`draw::ParticleDraw`), which the renderer
//! calls in its pass with the meshes.
use super::draw::ParticleDraw;
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
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
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
    right: [f32; 4],
    up: [f32; 4],
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
    /// Its group 0 for each renderer drawing it, by the drawer's prepare
    /// each was made for, the newest first.
    draw_binds: Vec<(u64, wgpu::BindGroup)>,
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

/// Pipelines made on a thread of their own with the first frame (none
/// where the device runs no compute).
struct Pipelines {
    update: Option<wgpu::ComputePipeline>,
    /// `update` giving birth into a sub-emitter's pool.
    update_events: Option<wgpu::ComputePipeline>,
}
type Compiling = Arc<Mutex<Option<Pipelines>>>;

/// A world's particles on one device (`Particles::devices`): its pools and
/// update pipelines, shared by every painter drawing it there.
pub(crate) struct Shared {
    compute: bool,
    update_layout: Option<wgpu::BindGroupLayout>,
    events_layout: Option<wgpu::BindGroupLayout>,
    compiling: Option<Compiling>,
    pipelines: Option<Pipelines>,
    pools: HashMap<u64, Pool>,
    /// Steps taken on this device.
    index: u64,
}

impl Shared {
    fn new(device: &wgpu::Device) -> Self {
        let mut shared = Self {
            compute: crate::Caps::of(device).compute,
            update_layout: None,
            events_layout: None,
            compiling: None,
            pipelines: None,
            pools: HashMap::new(),
            index: 0,
        };
        shared.compiling = Some(shared.compile(device));
        shared
    }

    /// Whether its update pipelines are made (taking them if just done).
    fn ready(&mut self) -> bool {
        if let Some(compiling) = &self.compiling {
            let done = compiling.lock().unwrap_or_else(|p| p.into_inner()).take();
            if let Some(done) = done {
                self.pipelines = Some(done);
                self.compiling = None;
            }
        }
        self.pipelines.is_some()
    }

    /// The world's emitters stepped `step` seconds on: spawned, updated (on
    /// the GPU, recorded in `frame`'s encoder, or on the CPU).
    fn step(
        &mut self,
        particles: &mut Particles,
        inner: &Inner,
        frame: &mut ViewportFrame<'_>,
        step: f32,
    ) {
        let device = frame.device;
        particles.time += f64::from(step);
        particles.steps = particles.steps.wrapping_add(1);
        let time = particles.time;
        let Some(pipelines) = &self.pipelines else {
            return;
        };
        let on_gpu = self.compute && !particles.on_cpu && pipelines.update.is_some();
        let Some(update_layout) = &self.update_layout else {
            return;
        };
        // The camera's right and up in the world: its view's first rows.
        let view = inner.camera.view().columns;
        let right = [view[0][0], view[1][0], view[2][0], 0.];
        let up = [view[0][1], view[1][1], view[2][1], 0.];
        self.index += 1;
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
        // Where the CPU steps them, the world's emitters in the order added
        // hold at most `CPU_WORLD_PARTICLES` together: one past it waits.
        let mut cpu_held = 0u32;
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
            if !on_gpu {
                let over = cpu_held.saturating_add(slots) > super::CPU_WORLD_PARTICLES;
                if over && !live.over_cpu {
                    eprintln!(
                        "zgx-3d: an emitter of {slots} particles is past the world's {} on the CPU: not stepped",
                        super::CPU_WORLD_PARTICLES
                    );
                }
                live.over_cpu = over;
                if over {
                    continue;
                }
                cpu_held += slots;
            } else {
                live.over_cpu = false;
            }
            let pool = self
                .pools
                .entry(live.id)
                .or_insert_with(|| make_pool(device, update_layout, slots, on_gpu));
            // Made for the other way of updating: made again, from this frame.
            if pool.cpu.is_some() == on_gpu || pool.slots != slots {
                *pool = make_pool(device, update_layout, slots, on_gpu);
            }
            particles.pools.push((pool.slots, pool.cpu.is_some()));
            pool.used = index;
            pool.blend = live.emitter.blend;
            if pool.failed {
                live.failed = true;
                pool.shown = false;
                continue;
            }
            let (base, count) = live.window(time, step, slots);
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
                None => {
                    frame
                        .queue
                        .write_buffer(&pool.update, 0, bytemuck::bytes_of(&u));
                    // A frame's births into it start again from none.
                    if live.fed
                        && let Some(born) = &pool.born
                    {
                        frame.queue.write_buffer(born, 4, &[0; 4]);
                    }
                }
            }
            let e = &live.emitter;
            let draw = DrawUniforms {
                right,
                up,
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

    /// `drawer`'s list for its current painter: each shown pool with its
    /// binding for that painter's renderer (made on `device`, where the
    /// pools are).
    fn hand_to(&mut self, particles: &Particles, drawer: &mut ParticleDraw, device: &wgpu::Device) {
        let Some((made, layout, uniforms)) = drawer.binding_on(device) else {
            return;
        };
        for at in particles.order() {
            let Some(pool) = particles.emitters[at]
                .1
                .as_ref()
                .and_then(|live| self.pools.get_mut(&live.id))
            else {
                continue;
            };
            if !pool.shown || pool.failed {
                continue;
            }
            let bind = match pool.draw_binds.iter().find(|(m, _)| *m == made) {
                Some((_, bind)) => bind.clone(),
                None => {
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("3D particles draw"),
                        layout: &layout,
                        entries: &[entry(0, &uniforms), entry(1, &pool.draw)],
                    });
                    // One a renderer drawing it: the newest few kept.
                    pool.draw_binds.insert(0, (made, bind.clone()));
                    pool.draw_binds.truncate(MOST_RENDERERS);
                    bind
                }
            };
            drawer.drawn.push(super::draw::Drawn {
                particles: pool.particles.clone(),
                slots: pool.slots,
                bind,
                made,
                blend: pool.blend,
            });
        }
    }

    /// The update pipelines, made on a thread of their own.
    fn compile(&mut self, device: &wgpu::Device) -> Compiling {
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
            *done.lock().unwrap_or_else(|p| p.into_inner()) = Some(Pipelines {
                update,
                update_events,
            });
        };
        if let Err(error) = crate::compute::compile_off_the_frame(make) {
            eprintln!("zgx-3d: particles' pipelines can't be made ({error}): none updated");
        }
        slot
    }
}

/// The renderers a pool keeps a binding for, at most (`World::view` and
/// a few `World::viewport`s).
const MOST_RENDERERS: usize = 4;

/// A painter's side of its world's particles: who it is to the world's
/// drawer.
pub(crate) struct Device {
    id: u64,
}
impl Default for Device {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl Device {
    /// Its device gone: what the world made on it is let go (by every
    /// painter: wgpu tells devices apart by id, and a new device's can be
    /// the gone one's).
    pub(crate) fn device_changed(&mut self, world: &Weak<RefCell<Inner>>) {
        self.forget(world, true);
    }
    /// Gone: the drawer keeps nothing of its renderer's.
    pub(crate) fn dropped(&mut self, world: &Weak<RefCell<Inner>>) {
        self.forget(world, false);
    }
    fn forget(&mut self, world: &Weak<RefCell<Inner>>, devices: bool) {
        let Some(world) = world.upgrade() else {
            return;
        };
        let Ok(mut inner) = world.try_borrow_mut() else {
            return;
        };
        if let Some(particles) = &mut inner.particles {
            particles.draw.borrow_mut().forget(self.id);
            if devices {
                particles.devices.clear();
            }
        }
    }

    /// Before the frame: the world's emitters stepped, if no painter has
    /// at this frame time (on the GPU, recorded in the frame's encoder, or
    /// on the CPU), and handed to the drawer for this painter's renderer.
    /// Whether the world has emitters (`after` is called only if so).
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
        let mut shared = match particles.devices.iter().position(|(d, _)| d == device) {
            Some(at) => particles.devices.remove(at).1,
            None => Shared::new(device),
        };
        let drawer = particles.draw.clone();
        let mut drawer = drawer.borrow_mut();
        drawer.current = self.id;
        drawer.drawn.clear();
        // Stepped once it can be drawn (the drawer made for this painter's
        // renderer, a frame after its first): its clock starts then. The
        // world steps once a frame time, by the first painter ready to; at
        // the same time again, it's only drawn. A painter not ready yet
        // leaves the time to the next.
        if shared.ready() && drawer.ready() {
            particles.ready = true;
            if particles.clock != Some(frame.time) {
                let step = particles.clock.map_or(0., |last| {
                    frame
                        .time
                        .saturating_duration_since(last)
                        .as_secs_f32()
                        .min(MAX_STEP)
                });
                particles.clock = Some(frame.time);
                shared.step(particles, inner, frame, step);
            }
            shared.hand_to(particles, &mut drawer, device);
        }
        particles.devices.insert(0, (device.clone(), shared));
        particles.devices.truncate(2);
    }

    /// After the frame: whether any particle keeps the world drawing.
    #[cold]
    #[inline(never)]
    pub(crate) fn after(&mut self, world: &Weak<RefCell<Inner>>) {
        let Some(world) = world.upgrade() else {
            return;
        };
        let inner = world.borrow();
        let unsettled = inner.particles.as_ref().is_some_and(|p| p.unsettled());
        inner.unsettle(UNSETTLED, unsettled);
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

fn make_pool(
    device: &wgpu::Device,
    update_layout: &wgpu::BindGroupLayout,
    slots: u32,
    on_gpu: bool,
) -> Pool {
    // Checked against the device's limits when the emitter was added; a
    // refusal still left here, not to the device's handler.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pool = make_pool_in_scope(device, update_layout, slots, on_gpu);
    if let Some(error) = crate::compute::now(scope.pop()) {
        eprintln!("zgx-3d: a particle pool of {slots} couldn't be made: {error}");
        pool.failed = true;
    }
    pool
}

fn make_pool_in_scope(
    device: &wgpu::Device,
    update_layout: &wgpu::BindGroupLayout,
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
    Pool {
        slots,
        particles,
        update,
        draw,
        update_bind,
        draw_binds: Vec::new(),
        cpu: (!on_gpu).then(|| vec![Particle::default(); slots as usize]),
        born: on_gpu.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("3D particles born"),
                size: 8,
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
