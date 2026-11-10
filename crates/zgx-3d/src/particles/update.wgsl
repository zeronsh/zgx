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
}

@group(0) @binding(0) var<uniform> emitter: Emitter;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;

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

fn spawn(slot: u32) -> Particle {
    var p: Particle;
    var state = pcg(slot ^ bitcast<u32>(emitter.life.w));
    p.seed = state;
    let kind = emitter.spawn.x;
    var direction = vec3(0.0);
    if kind == 1u {
        p.position = mix(emitter.a.xyz, emitter.b.xyz, vec3(random(&state), random(&state), random(&state)));
    } else if kind == 2u {
        p.position = emitter.a.xyz + in_sphere(&state) * emitter.a.w;
    } else if kind == 3u {
        p.position = emitter.a.xyz;
        // A direction within the cone's angle of its axis, uniform on the cap.
        let axis = normalize(emitter.b.xyz);
        let cos_t = mix(1.0, cos(emitter.a.w), random(&state));
        let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
        let phi = random(&state) * 6.2831853;
        let helper = select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(axis.x) > 0.9);
        let u = normalize(cross(axis, helper));
        let v = cross(axis, u);
        direction = axis * cos_t + (u * cos(phi) + v * sin(phi)) * sin_t;
    } else {
        p.position = emitter.a.xyz;
    }
    if kind == 3u {
        p.velocity = direction * length(emitter.velocity.xyz);
    } else {
        p.velocity = emitter.velocity.xyz;
    }
    p.velocity += in_sphere(&state) * emitter.velocity.w;
    p.life = mix(emitter.life.x, emitter.life.y, random(&state));
    p.size = mix(emitter.collide.z, emitter.collide.w, random(&state));
    p.color = emitter.color;
    p.flags = 0u;
    // Born at a random moment within the frame, so a stream doesn't band.
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

@compute @workgroup_size(256)
fn update(@builtin(global_invocation_id) id: vec3<u32>) {
    let slot = id.x;
    let slots = emitter.spawn.w;
    if slot >= slots {
        return;
    }
    let ring = (slot + slots - emitter.spawn.y) % slots;
    if ring < emitter.spawn.z {
        particles[slot] = spawn(slot);
        return;
    }
    var p = particles[slot];
    if p.age >= p.life {
        return;
    }
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
    particles[slot] = p;
}
