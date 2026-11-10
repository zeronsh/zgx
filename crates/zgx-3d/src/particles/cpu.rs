//! Particles updated on the CPU, where the device runs no compute (tier B),
//! up to `CPU_PARTICLES` an emitter: `update.wgsl`'s step in Rust, from the
//! same numbers, uploaded as the instances it draws.
use super::gpu::Uniforms;

/// A particle's bytes.
pub(crate) const PARTICLE_BYTES: u64 = std::mem::size_of::<Particle>() as u64;

/// A particle as the GPU holds it (`Particle` in `update.wgsl`): 80 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Particle {
    pub(crate) position: [f32; 3],
    pub(crate) age: f32,
    pub(crate) velocity: [f32; 3],
    pub(crate) life: f32,
    pub(crate) color: [f32; 4],
    pub(crate) size: f32,
    pub(crate) seed: u32,
    pub(crate) flags: u32,
    pub(crate) _pad: u32,
    pub(crate) app: [f32; 4],
}

fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}
fn random(state: &mut u32) -> f32 {
    *state = pcg(*state);
    (*state >> 8) as f32 * (1. / 16_777_216.)
}
fn in_sphere(state: &mut u32) -> [f32; 3] {
    let z = random(state) * 2. - 1.;
    let t = random(state) * std::f32::consts::TAU;
    let r = (1. - z * z).max(0.).sqrt();
    let s = random(state).cbrt();
    [r * t.cos() * s, r * t.sin() * s, z * s]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|x| x * s)
}
fn xyz(v: [f32; 4]) -> [f32; 3] {
    [v[0], v[1], v[2]]
}

/// A particle of `u`'s born at `slot`, its spawn shape placed at `origin`
/// (`spawn` in `update.wgsl`).
pub(crate) fn spawn(u: &Uniforms, slot: u32, seed: u32, origin: [f32; 3]) -> Particle {
    let mut state = pcg(slot ^ seed);
    let mut p = Particle {
        seed: state,
        ..Default::default()
    };
    let mut direction = [0.; 3];
    match u.spawn[0] {
        1 => {
            let t = [random(&mut state), random(&mut state), random(&mut state)];
            p.position = [0, 1, 2].map(|i| u.a[i] + (u.b[i] - u.a[i]) * t[i]);
        }
        2 => p.position = add(xyz(u.a), scale(in_sphere(&mut state), u.a[3])),
        3 => {
            p.position = xyz(u.a);
            let axis = normalize(xyz(u.b));
            let cos_t = 1. + (u.a[3].cos() - 1.) * random(&mut state);
            let sin_t = (1. - cos_t * cos_t).max(0.).sqrt();
            let phi = random(&mut state) * std::f32::consts::TAU;
            let helper = if axis[0].abs() > 0.9 {
                [0., 1., 0.]
            } else {
                [1., 0., 0.]
            };
            let a = normalize(cross(axis, helper));
            let b = cross(axis, a);
            direction = add(
                scale(axis, cos_t),
                add(scale(a, phi.cos() * sin_t), scale(b, phi.sin() * sin_t)),
            );
        }
        _ => p.position = xyz(u.a),
    }
    p.position = add(p.position, origin);
    p.velocity = if u.spawn[0] == 3 {
        scale(direction, length(xyz(u.velocity)))
    } else {
        xyz(u.velocity)
    };
    p.velocity = add(p.velocity, scale(in_sphere(&mut state), u.velocity[3]));
    p.life = u.life[0] + (u.life[1] - u.life[0]) * random(&mut state);
    p.size = u.collide[2] + (u.collide[3] - u.collide[2]) * random(&mut state);
    p.color = u.color;
    p
}

/// Born in this frame's slice of the ring, at a random moment in the frame
/// (`spawn_here` in `update.wgsl`).
fn spawn_here(slot: u32, u: &Uniforms) -> Particle {
    let mut p = spawn(u, slot, u.life[3].to_bits(), [0.; 3]);
    let mut state = pcg(p.seed ^ 0x5bd1_e995);
    p.age = random(&mut state) * u.life[2];
    p.position = add(p.position, scale(p.velocity, p.age));
    p
}

/// What happened to a particle this frame, for its sub-emitter.
pub(crate) const DIED: u32 = 1;
pub(crate) const HIT: u32 = 2;

/// A sub-emitter's particles born in `children` (`born` so far), each
/// `(position, how many)` of `events`, by `child`'s numbers, from parents'
/// slots (`update_events` in `update.wgsl`).
pub(crate) fn give_birth(
    children: &mut [Particle],
    born: &mut u32,
    child: &Uniforms,
    events: &[(u32, [f32; 3], u32)],
) {
    let slots = children.len() as u32;
    for &(parent, at, births) in events {
        let seed = child.life[3].to_bits() ^ pcg(parent);
        for k in 0..births {
            let slot = born.wrapping_add(k) % slots;
            children[slot as usize] = spawn(child, slot, seed ^ k.wrapping_mul(0x9e37_79b9), at);
        }
        *born = born.wrapping_add(births);
    }
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a);
    if l > 0. {
        scale(a, 1. / l)
    } else {
        [0., 1., 0.]
    }
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `particles` a frame on by `u`, as `update.wgsl` steps them: whether any
/// changed (a pool all dead and spawning none is left as it was). Births its
/// deaths and hits give a sub-emitter (`u.events`) go into `events`: the
/// slot, where, and how many.
pub(crate) fn step(
    particles: &mut [Particle],
    u: &Uniforms,
    events: &mut Vec<(u32, [f32; 3], u32)>,
) -> bool {
    let slots = particles.len() as u32;
    let dt = u.life[2];
    let mut changed = false;
    for (slot, p) in (0..slots).zip(particles.iter_mut()) {
        let ring = (slot + slots - u.spawn[1] % slots) % slots;
        if ring < u.spawn[2] {
            *p = spawn_here(slot, u);
            changed = true;
            continue;
        }
        if p.age >= p.life {
            continue;
        }
        changed = true;
        let mut happened = 0;
        if p.flags == 0 {
            let mut a = add(
                xyz(u.accel),
                scale([0, 1, 2].map(|i| u.wind[i] - p.velocity[i]), u.accel[3]),
            );
            if u.wind[3] != 0. {
                let q = scale(p.position, u.swirl[0]);
                let t = u.swirl[1];
                a = add(
                    a,
                    scale(
                        [
                            (q[1] + t).sin() - (q[2] * 1.3).cos(),
                            (q[2] + t * 1.1).sin() - (q[0] * 1.3).cos(),
                            (q[0] + t * 0.9).sin() - (q[1] * 1.3).cos(),
                        ],
                        u.wind[3],
                    ),
                );
            }
            p.velocity = add(p.velocity, scale(a, dt));
            p.position = add(p.position, scale(p.velocity, dt));
            let action = u.collide[0] as u32;
            if action != 0 && p.position[1] < u.swirl[2] && p.velocity[1] < 0. {
                happened = HIT;
                match action {
                    1 => {
                        p.position[1] = u.swirl[2];
                        let f = 1. - u.collide[1];
                        p.velocity = [
                            p.velocity[0] * f,
                            -p.velocity[1] * u.swirl[3],
                            p.velocity[2] * f,
                        ];
                    }
                    2 => {
                        p.position[1] = u.swirl[2];
                        p.velocity = [0.; 3];
                        p.flags = 1;
                    }
                    _ => p.age = p.life,
                }
            }
        }
        p.age += dt;
        if p.age >= p.life {
            happened |= DIED;
        }
        let births = (if happened & DIED != 0 { u.events[1] } else { 0 })
            + (if happened & HIT != 0 { u.events[2] } else { 0 });
        if births > 0 && u.events[0] > 0 {
            events.push((slot, p.position, births));
        }
    }
    changed
}
