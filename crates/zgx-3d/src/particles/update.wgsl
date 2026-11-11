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
// Its births: in all (where the next goes in its ring), and this frame
// (none past its slots).
@group(0) @binding(4) var<storage, read_write> born: array<atomic<u32>, 2>;
// The frame's births, in the parents' order (not the order invocations
// happen to run in, which differs run to run): each parent's count
// (`update_events`), then for each 256 of them their sum, then where their
// births begin (`events_offsets`), and last the ring's head this frame.
// One buffer, so a device binding four storage buffers to a stage has
// room: a slot's count at its index, a group's at the slots' count past.
@group(0) @binding(5) var<storage, read_write> scratch: array<u32>;

var<workgroup> scan: array<u32, 256>;

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

// Births a parent's `happened` gives, at most its sub-emitter's slots.
fn births_of(happened: u32) -> u32 {
    let most = emitter.events.x;
    var births = 0u;
    if (happened & DIED) != 0u {
        births = min(emitter.events.y, most);
    }
    if (happened & HIT) != 0u {
        births = min(births + min(emitter.events.z, most), most);
    }
    return births;
}

// `scan` summed from its first to each entry (inclusive), each sum at most
// `most`. Every invocation of the workgroup calls it.
fn scan_up(local: u32, most: u32) {
    for (var stride = 1u; stride < 256u; stride <<= 1u) {
        var add = 0u;
        if local >= stride {
            add = scan[local - stride];
        }
        workgroupBarrier();
        scan[local] = min(scan[local] + add, most);
        workgroupBarrier();
    }
}

// As `update`, each particle's births this frame counted (its deaths and
// hits, as many as `emitter.events` says), and each 256's in all.
@compute @workgroup_size(256)
fn update_events(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) group: vec3<u32>,
) {
    var births = 0u;
    if id.x < emitter.spawn.w {
        births = births_of(step(id.x));
        scratch[id.x] = births;
    }
    scan[local] = births;
    workgroupBarrier();
    scan_up(local, emitter.events.x);
    if local == 255u {
        scratch[emitter.spawn.w + group.x] = scan[255];
    }
}

// One workgroup: where each 256 parents' births begin, in their order, and
// the ring's head this frame (`groups`' last), the head moved on by the
// frame's births (a pool's worth at most).
@compute @workgroup_size(256)
fn events_offsets(@builtin(local_invocation_index) local: u32) {
    let most = emitter.events.x;
    let n = (emitter.spawn.w + 255u) / 256u;
    let per = (n + 255u) / 256u;
    let first = local * per;
    let last = min(first + per, n);
    var sum = 0u;
    for (var g = first; g < last; g++) {
        sum = min(sum + scratch[emitter.spawn.w + g], most);
    }
    scan[local] = sum;
    workgroupBarrier();
    scan_up(local, most);
    var at = 0u;
    if local > 0u {
        at = scan[local - 1u];
    }
    for (var g = first; g < last; g++) {
        let count = scratch[emitter.spawn.w + g];
        scratch[emitter.spawn.w + g] = at;
        at = min(at + count, most);
    }
    if local == 255u {
        let head = atomicLoad(&born[0]);
        scratch[emitter.spawn.w + n] = head;
        atomicStore(&born[0], (head + scan[255]) % most);
    }
}

// Each parent's births written into its sub-emitter's ring, in the
// parents' order from the frame's head: none past a pool's worth.
@compute @workgroup_size(256)
fn events_births(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) group: vec3<u32>,
) {
    let most = emitter.events.x;
    var births = 0u;
    if id.x < emitter.spawn.w {
        births = scratch[id.x];
    }
    scan[local] = births;
    workgroupBarrier();
    scan_up(local, most);
    var before = 0u;
    if local > 0u {
        before = scan[local - 1u];
    }
    if births == 0u {
        return;
    }
    let n = (emitter.spawn.w + 255u) / 256u;
    let start = min(scratch[emitter.spawn.w + group.x] + before, most);
    let head = scratch[emitter.spawn.w + n];
    let p = particles[id.x];
    let seed = bitcast<u32>(child.life.w) ^ pcg(id.x);
    for (var k = 0u; k < births && start + k < most; k++) {
        let slot = (head + start + k) % most;
        children[slot] = spawn(child, slot, seed ^ (k * 0x9e3779b9u), p.position);
    }
}
