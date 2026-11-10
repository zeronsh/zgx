// An emitter's particles a frame (particles/mod.rs): those in this frame's
// slice of the ring spawned, the rest alive moved by the emitter's forces,
// aged, and collided. One invocation a slot.

struct Particle {
    position: vec3<f32>,
    age: f32,
    velocity: vec3<f32>,
    life: f32,
    color: vec4<f32>,
    size: f32,
    seed: u32,
    flags: u32,
    _pad: u32,
    app: vec4<f32>,
}

// `Uniforms` in particles/gpu.rs.
struct Emitter {
    // Spawn shape (0 point, 1 box, 2 sphere, 3 cone), the ring's first slot
    // spawned this frame, how many, the slots in all.
    spawn: vec4<u32>,
    // The shape's numbers: a box's corners; a sphere's centre and radius; a
    // cone's apex and angle, then its axis.
    a: vec4<f32>,
    b: vec4<f32>,
    // Velocity: a base, and how far a random part reaches.
    velocity: vec4<f32>,
    // Lifetime's least and most, the step, and the frame's seed (bits).
    life: vec4<f32>,
    // Constant acceleration (gravity and others), and drag.
    accel: vec4<f32>,
    // What drag pulls velocity toward (wind), and swirl's strength.
    wind: vec4<f32>,
    // Swirl's scale and time; a collision plane's height, its bounce.
    swirl: vec4<f32>,
    // Collision: 0 none, 1 bounce, 2 stick, 3 die; friction; size's least
    // and most at spawn.
    collide: vec4<f32>,
    // The start colour (linear, premultiplied by nothing).
    color: vec4<f32>,
    // Its sub-emitter's: its slots, born a particle's death, born a hit
    // (`update_events` only).
    events: vec4<u32>,
}

@group(0) @binding(0) var<uniform> emitter: Emitter;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
// A sub-emitter's numbers, pool and count of particles born into it, for
// `update_events`: its parents write its births as their events happen.
@group(0) @binding(2) var<uniform> child: Emitter;
@group(0) @binding(3) var<storage, read_write> children: array<Particle>;
@group(0) @binding(4) var<storage, read_write> born: atomic<u32>;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn random(state: ptr<function, u32>) -> f32 {
    *state = pcg(*state);
    return f32(*state >> 8u) * (1.0 / 16777216.0);
}
fn in_sphere(state: ptr<function, u32>) -> vec3<f32> {
    let z = random(state) * 2.0 - 1.0;
    let t = random(state) * 6.2831853;
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(t), r * sin(t), z) * pow(random(state), 1.0 / 3.0);
}

// A particle of `e` born at `slot`, its spawn shape placed at `origin`.
fn spawn(e: Emitter, slot: u32, seed: u32, origin: vec3<f32>) -> Particle {
    var p: Particle;
    var state = pcg(slot ^ seed);
    p.seed = state;
    let kind = e.spawn.x;
    var direction = vec3(0.0);
    if kind == 1u {
        p.position = mix(e.a.xyz, e.b.xyz, vec3(random(&state), random(&state), random(&state)));
    } else if kind == 2u {
        p.position = e.a.xyz + in_sphere(&state) * e.a.w;
    } else if kind == 3u {
        p.position = e.a.xyz;
        // A direction within the cone's angle of its axis, uniform on the cap.
        let axis = normalize(e.b.xyz);
        let cos_t = mix(1.0, cos(e.a.w), random(&state));
        let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
        let phi = random(&state) * 6.2831853;
        let helper = select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(axis.x) > 0.9);
        let u = normalize(cross(axis, helper));
        let v = cross(axis, u);
        direction = axis * cos_t + (u * cos(phi) + v * sin(phi)) * sin_t;
    } else {
        p.position = e.a.xyz;
    }
    p.position += origin;
    if kind == 3u {
        p.velocity = direction * length(e.velocity.xyz);
    } else {
        p.velocity = e.velocity.xyz;
    }
    p.velocity += in_sphere(&state) * e.velocity.w;
    p.life = mix(e.life.x, e.life.y, random(&state));
    p.size = mix(e.collide.z, e.collide.w, random(&state));
    p.color = e.color;
    p.flags = 0u;
    return p;
}

// A particle of this emitter's born in this frame's slice of its ring: at a
// random moment within the frame, so a stream doesn't band.
fn spawn_here(slot: u32) -> Particle {
    var p = spawn(emitter, slot, bitcast<u32>(emitter.life.w), vec3(0.0));
    var state = pcg(p.seed ^ 0x5bd1e995u);
    p.age = random(&state) * emitter.life.z;
    p.position += p.velocity * p.age;
    return p;
}

fn swirl(at: vec3<f32>) -> vec3<f32> {
    let s = emitter.swirl.x;
    let t = emitter.swirl.y;
    let q = at * s;
    return vec3(
        sin(q.y + t) - cos(q.z * 1.3),
        sin(q.z + t * 1.1) - cos(q.x * 1.3),
        sin(q.x + t * 0.9) - cos(q.y * 1.3),
    ) * emitter.wind.w;
}

// What happened to a particle this frame, for its sub-emitter.
const NOTHING: u32 = 0u;
const DIED: u32 = 1u;
const HIT: u32 = 2u;

// Slot `slot` a frame on: spawned, or moved, collided and aged. What
// happened to it, its slot written.
fn step(slot: u32) -> u32 {
    let slots = emitter.spawn.w;
    let ring = (slot + slots - emitter.spawn.y) % slots;
    if ring < emitter.spawn.z {
        particles[slot] = spawn_here(slot);
        return NOTHING;
    }
    var p = particles[slot];
    if p.age >= p.life {
        return NOTHING;
    }
    var happened = NOTHING;
    let dt = emitter.life.z;
    if p.flags == 0u {
        var a = emitter.accel.xyz + (emitter.wind.xyz - p.velocity) * emitter.accel.w;
        if emitter.wind.w != 0.0 {
            a += swirl(p.position);
        }
        p.velocity += a * dt;
        p.position += p.velocity * dt;
        let action = u32(emitter.collide.x);
        if action != 0u && p.position.y < emitter.swirl.z && p.velocity.y < 0.0 {
            happened = HIT;
            if action == 1u {
                p.position.y = emitter.swirl.z;
                p.velocity = vec3(p.velocity.x, -p.velocity.y * emitter.swirl.w, p.velocity.z) * vec3(1.0 - emitter.collide.y, 1.0, 1.0 - emitter.collide.y);
            } else if action == 2u {
                p.position.y = emitter.swirl.z;
                p.velocity = vec3(0.0);
                p.flags = 1u;
            } else {
                p.age = p.life;
            }
        }
    }
    p.age += dt;
    if p.age >= p.life {
        happened |= DIED;
    }
    particles[slot] = p;
    return happened;
}

@compute @workgroup_size(256)
fn update(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < emitter.spawn.w {
        step(id.x);
    }
}

// As `update`, its particles' deaths and hits giving birth in its
// sub-emitter's pool, as many each as `emitter.events` says.
@compute @workgroup_size(256)
fn update_events(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= emitter.spawn.w {
        return;
    }
    let happened = step(id.x);
    var births = 0u;
    if (happened & DIED) != 0u {
        births += emitter.events.y;
    }
    if (happened & HIT) != 0u {
        births += emitter.events.z;
    }
    if births == 0u {
        return;
    }
    let p = particles[id.x];
    let first = atomicAdd(&born, births);
    let seed = bitcast<u32>(child.life.w) ^ pcg(id.x);
    for (var k = 0u; k < births; k++) {
        let slot = (first + k) % emitter.events.x;
        children[slot] = spawn(child, slot, seed ^ (k * 0x9e3779b9u), p.position);
    }
}
