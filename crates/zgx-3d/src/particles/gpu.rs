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
    time::Instant,
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
    /// A pixel's width and height in clip space (2 / the frame's size),
    /// and the view depths a particle fades in between (`NEAR_FADE`).
    viewport: [f32; 4],
}

/// View depths (m) over which a particle fades in from the camera: none
/// nearer than the first, whole past the second.
const NEAR_FADE: (f32, f32) = (0.3, 1.5);

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
    /// Where it gives birth into a sub-emitter's on the GPU: each
    /// particle's births this frame (4 bytes each), then each 256's offset
    /// and the child's ring head (`update.wgsl`'s `scratch`).
    events: Option<wgpu::Buffer>,
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
    /// `update` giving birth into a sub-emitter's pool, in the parents'
    /// order: counting each parent's births, placing each 256's, writing
    /// them (`update_events`, `events_offsets`, `events_births`).
    events: Option<[wgpu::ComputePipeline; 3]>,
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
    /// The frame time it was last stepped at (`Particles::plan`'s).
    stepped: Option<Instant>,
}

/// What a frame's step does to each emitter's pool, planned once a frame
/// time (`Shared::plan`) and done by every device drawing the world
/// (`Shared::apply`), so each device's pools see the same spawns and steps.
#[derive(Default)]
pub(crate) struct Plan {
    /// The frame time it's for.
    at: Option<Instant>,
    /// Whether it was planned for pools the GPU updates (their slots are).
    on_gpu: bool,
    planned: Vec<Planned>,
}
/// One emitter's part of a `Plan`.
struct Planned {
    id: u64,
    slots: u32,
    blend: Blend,
    /// Whether any of its particles may live this frame (else neither
    /// updated nor drawn).
    shown: bool,
    /// Whether a parent gives birth into it (its births start again).
    fed: bool,
    /// Its sub-emitter, which its births go into.
    child: Option<u64>,
    update: Uniforms,
    draw: DrawUniforms,
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
            stepped: None,
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

    /// The world's emitters stepped `step` seconds on, once a frame time:
    /// spawned, aged, fed, held to the CPU's cap where it updates them.
    /// What each emitter's pool does this frame, for every device's
    /// (`apply`).
    fn plan(&self, particles: &mut Particles, inner: &Inner, at: Instant, step: f32) -> Plan {
        particles.time += f64::from(step);
        particles.steps = particles.steps.wrapping_add(1);
        let time = particles.time;
        let on_gpu = self.on_gpu(particles);
        // The camera's right and up in the world: its view's first rows.
        let view = inner.camera.view().columns;
        let right = [view[0][0], view[1][0], view[2][0], 0.];
        let up = [view[0][1], view[1][1], view[2][1], 0.];
        let seed_base = particles.steps.wrapping_mul(0x9e37_79b9);
        let slots_of = |live: &Live| {
            if on_gpu {
                live.emitter.max
            } else {
                live.emitter.max.min(CPU_PARTICLES)
            }
        };
        particles.feed();
        let mut planned = Vec::new();
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
                });
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
            let (base, count) = live.window(time, step, slots);
            let shown = !live.failed && (live.unsettled(time) || count > 0);
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
                viewport: [0.; 4],
            };
            planned.push(Planned {
                id: live.id,
                slots,
                blend: e.blend,
                shown,
                fed: live.fed,
                child: child.map(|c| c.0),
                update: u,
                draw,
            });
        }
        Plan {
            at: Some(at),
            on_gpu,
            planned,
        }
    }

    /// Whether this device updates `particles` on the GPU.
    fn on_gpu(&self, particles: &Particles) -> bool {
        self.compute
            && !particles.on_cpu
            && self.pipelines.as_ref().is_some_and(|p| p.update.is_some())
    }

    /// The frame's `plan` done to this device's pools (made as the plan
    /// needs them), recorded in `frame`'s encoder or done on the CPU.
    /// The emitters whose pools the device refused.
    fn apply(
        &mut self,
        plan: &Plan,
        particles: &Particles,
        frame: &mut ViewportFrame<'_>,
    ) -> Vec<u64> {
        let device = frame.device;
        let mut refused = Vec::new();
        let (Some(pipelines), Some(update_layout)) = (&self.pipelines, &self.update_layout) else {
            return refused;
        };
        // A device updating them the other way (the plan's slots are its
        // way's): this one's left as it was.
        let on_gpu = self.on_gpu(particles);
        if on_gpu != plan.on_gpu {
            return refused;
        }
        self.index += 1;
        let index = self.index;
        let mut pass: Option<wgpu::ComputePass<'static>> = None;
        let mut events: Vec<(u32, [f32; 3], u32)> = Vec::new();
        for planned in &plan.planned {
            let slots = planned.slots;
            let pool = self
                .pools
                .entry(planned.id)
                .or_insert_with(|| make_pool(device, update_layout, slots, on_gpu));
            // Made for the other way of updating: made again, from this frame.
            if pool.cpu.is_some() == on_gpu || pool.slots != slots {
                *pool = make_pool(device, update_layout, slots, on_gpu);
            }
            pool.used = index;
            pool.blend = planned.blend;
            if pool.failed {
                refused.push(planned.id);
                pool.shown = false;
                continue;
            }
            pool.shown = planned.shown;
            if !pool.shown {
                continue;
            }
            let u = planned.update;
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
                    if planned.fed
                        && let Some(born) = &pool.born
                    {
                        frame.queue.write_buffer(born, 4, &[0; 4]);
                    }
                }
            }
            frame.queue.write_buffer(
                &pool.draw,
                0,
                bytemuck::bytes_of(&DrawUniforms {
                    // This device's frame's pixels.
                    viewport: [
                        2. / frame.size.0.max(1) as f32,
                        2. / frame.size.1.max(1) as f32,
                        NEAR_FADE.0,
                        NEAR_FADE.1,
                    ],
                    ..planned.draw
                }),
            );
            let child = planned.child.filter(|id| self.pools.contains_key(id));
            // A parent's place to count its births, made with its first.
            if child.is_some()
                && let Some(pool) = self.pools.get_mut(&planned.id)
                && pool.cpu.is_none()
                && pool.events.is_none()
            {
                let words = u64::from(slots) + u64::from(slots.div_ceil(256)) + 1;
                pool.events = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D particles births"),
                    size: words * 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }));
            }
            let pool = &self.pools[&planned.id];
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
                let groups = slots.div_ceil(256);
                let feeding = child
                    .zip(pipelines.events.as_ref())
                    .zip(self.events_layout.as_ref())
                    .zip(pool.events.as_ref());
                match feeding {
                    Some((((child, [count, place, write]), layout), scratch)) => {
                        let into = &self.pools[&child];
                        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("3D particles giving birth"),
                            layout,
                            entries: &[
                                entry(0, &pool.update),
                                entry(1, &pool.particles),
                                entry(2, &into.update),
                                entry(3, &into.particles),
                                entry(4, into.born.as_ref().unwrap()),
                                entry(5, scratch),
                            ],
                        });
                        // Stepped and counted, placed in the parents'
                        // order, written: the same births in the same
                        // slots each run.
                        compute.set_bind_group(0, &bind, &[]);
                        compute.set_pipeline(count);
                        compute.dispatch_workgroups(groups, 1, 1);
                        compute.set_pipeline(place);
                        compute.dispatch_workgroups(1, 1, 1);
                        compute.set_pipeline(write);
                        compute.dispatch_workgroups(groups, 1, 1);
                    }
                    None => {
                        compute.set_pipeline(pipelines.update.as_ref().unwrap());
                        compute.set_bind_group(0, pool.update_bind.as_ref().unwrap(), &[]);
                        compute.dispatch_workgroups(groups, 1, 1);
                    }
                }
            } else if let Some(child) = child
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
        refused
    }

    /// `drawer`'s list for its current painter: each shown pool with its
    /// binding for that painter's renderer (made on `device`, where the
    /// pools are).
    fn hand_to(
        &mut self,
        particles: &Particles,
        drawer: &mut ParticleDraw,
        device: &wgpu::Device,
        id: u64,
    ) {
        let Some((made, layout, uniforms)) = drawer.binding_on(id) else {
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
                        layout_entry(5, wgpu::ShaderStages::COMPUTE, storage),
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
            let events = (|| {
                Some([
                    update_pipeline(&events_layout, "update_events")?,
                    update_pipeline(&events_layout, "events_offsets")?,
                    update_pipeline(&events_layout, "events_births")?,
                ])
            })();
            *done.lock().unwrap_or_else(|p| p.into_inner()) = Some(Pipelines { update, events });
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
    /// Its device gone: its renderer's prepare let go, and what the world
    /// made for frames that carry no device identity (0: such a device's
    /// next could be taken for it). Another device's, identified, stay:
    /// none is ever the gone one's, and a gone one's falls off as others
    /// come (`Particles::devices`).
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
                particles.devices.retain(|(id, _)| *id != 0);
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
        // By the device's identity, never its equality (which holds across
        // wgpu instances).
        let id = frame.device_id();
        // A device without an identity (0: a frame made with
        // `ViewportFrame::new` and given none) beside one with: the two would
        // be told apart only by luck. One a painter of its own draws with
        // takes one (`viewport::new_device_id`).
        debug_assert!(
            particles
                .devices
                .iter()
                .all(|(d, _)| (*d == 0) == (id == 0)),
            "a world drawn with a device of no identity (0) beside one with ({id}): give it one, `zgx_gpu::viewport::new_device_id`"
        );
        let mut shared = match particles.devices.iter().position(|(d, _)| *d == id) {
            Some(at) => particles.devices.remove(at).1,
            None => Shared::new(device),
        };
        let drawer = particles.draw.clone();
        let mut drawer = drawer.borrow_mut();
        drawer.current = (self.id, id);
        drawer.drawn.clear();
        // Stepped once it can be drawn (the drawer made for this painter's
        // renderer, a frame after its first): its clock starts then. The
        // world steps once a frame time, by the first painter ready to; at
        // the same time again, it's only drawn. A painter not ready yet
        // leaves the time to the next.
        if shared.ready() && drawer.ready() {
            particles.ready = true;
            let planned = particles.clock != Some(frame.time);
            if planned {
                let step = particles.clock.map_or(0., |last| {
                    frame
                        .time
                        .saturating_duration_since(last)
                        .as_secs_f32()
                        .min(MAX_STEP)
                });
                particles.clock = Some(frame.time);
                particles.plan = shared.plan(particles, inner, frame.time, step);
            }
            // Each device's pools step once a frame time too: by the plan
            // the first made, so every device's see the same.
            if shared.stepped != Some(frame.time) && particles.plan.at == Some(frame.time) {
                shared.stepped = Some(frame.time);
                let plan = std::mem::take(&mut particles.plan);
                let refused = shared.apply(&plan, particles, frame);
                particles.plan = plan;
                for id in refused {
                    if let Some(live) = particles.get_mut_by_id(id) {
                        live.failed = true;
                    }
                }
                if planned {
                    particles.pools = particles
                        .order()
                        .into_iter()
                        .filter_map(|at| shared.pools.get(&particles.emitters[at].1.as_ref()?.id))
                        .map(|pool| (pool.slots, pool.cpu.is_some()))
                        .collect();
                }
            }
            shared.hand_to(particles, &mut drawer, device, id);
        }
        particles.devices.insert(0, (id, shared));
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
        events: None,
        shown: false,
        blend: Blend::Alpha,
        used: 0,
    }
}
