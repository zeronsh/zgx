// Probes updated by rays (`probes.rs`): this frame's probes each cast
// `trace.rays` rays (a spherical Fibonacci set, turned each round) into the
// scene's acceleration structure; each hit is lit by the sun and point
// lights (shadowed by rays of their own), by the probes' light from the
// last update (so light bounces on), and its own glow; a miss sees the
// sky. Each ray's light and distance go to `rays` for `probes_blend.wgsl`.
// Joined after the frame's declarations (`frame.wgsl`) and the probes'
// (`shader/probes.wgsl`), their tags taken off.

struct Trace {
    // Turns this round's rays.
    turn: mat4x4<f32>,
    // The first probe updated (of all volumes', in order), how many, rays
    // a probe, and how many probes there are.
    first: u32,
    count: u32,
    rays: u32,
    total: u32,
    // The farthest a ray's distance counts, and how far off a surface a
    // shadow ray starts.
    far: f32,
    lift: f32,
    // Point and spot lights in `lights`; the round.
    lights: u32,
    seed: u32,
}
// A mesh in the scene's acceleration structure (`raytrace.rs`): its
// colour and metalness, its glow, and where its vertices and indices start.
struct RayInstance {
    albedo: vec4<f32>,
    emissive: vec4<f32>,
    geometry: vec4<u32>,
}
@group(0) @binding(16) var scene: acceleration_structure;
// Every geometry's vertices (`Vertex`: position, normal, texture
// coordinate; 8 floats) and indices, one after another.
@group(0) @binding(17) var<storage, read> vertices: array<f32>;
@group(0) @binding(18) var<storage, read> indices: array<u32>;
@group(0) @binding(19) var<storage, read> instances: array<RayInstance>;
@group(0) @binding(20) var<storage, read_write> rays: array<vec4<f32>>;
@group(0) @binding(21) var<uniform> trace: Trace;
// Every point and spot light (`trace_lights`): place and range, colour
// and intensity, cone; not the frame's `points`, which are the eight
// nearest the camera's target.
@group(0) @binding(22) var<storage, read> lights: array<array<vec4<f32>, 3>>;

const PI: f32 = 3.14159265;

// The `i`th of `n` directions spread evenly over the sphere.
fn fibonacci(i: u32, n: u32) -> vec3<f32> {
    let phi = 2.0 * PI * fract(f32(i) * 0.618034);
    let z = 1.0 - (2.0 * f32(i) + 1.0) / f32(n);
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}
fn ray_direction(i: u32) -> vec3<f32> {
    return normalize((trace.turn * vec4(fibonacci(i, trace.rays), 0.0)).xyz);
}
// Probe `index`'s place (of all volumes', in order).
fn probe_place(index: u32) -> vec3<f32> {
    for (var i = 0u; i < probe_set.count.x; i++) {
        let volume = probe_set.volumes[i];
        let first = u32(volume.origin.w);
        let n = volume.counts.x * volume.counts.y * volume.counts.z;
        if index >= first && index < first + n {
            let local = index - first;
            let at = vec3(local % volume.counts.x, (local / volume.counts.x) % volume.counts.y, local / (volume.counts.x * volume.counts.y));
            return volume.origin.xyz + vec3<f32>(at) * volume.spacing.xyz;
        }
    }
    return vec3(0.0);
}
// Whether anything lies between `from` and `distance` along `d`.
fn blocked(start: vec3<f32>, d: vec3<f32>, distance: f32) -> bool {
    var q: ray_query;
    rayQueryInitialize(&q, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xffu, 0.0, distance, start, d));
    rayQueryProceed(&q);
    return rayQueryGetCommittedIntersection(&q).kind != RAY_QUERY_INTERSECTION_NONE;
}
// The light around the scene along `d`: the environment's, or the
// hemisphere's sky above and ground below.
fn sky_along(d: vec3<f32>) -> vec3<f32> {
    var light = select(frame.ground.rgb, frame.sky.rgb, d.y >= 0.0);
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     let turned = vec3(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
//E     light = textureSampleLevel(environment_cube, environment_sampler, turned, 1.0).rgb * frame.environment.x;
    return light;
}
// The diffuse light over π the world gives a surface facing `n` where no
// probe reaches: the hemisphere's or environment's.
fn around(n: vec3<f32>) -> vec3<f32> {
    var light = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     let t = vec3(c * n.x - s * n.z, n.y, s * n.x + c * n.z);
//E     let k = frame.irradiance;
//E     light = (k[0].rgb + k[1].rgb * t.y + k[2].rgb * t.z + k[3].rgb * t.x
//E         + k[4].rgb * (t.x * t.y) + k[5].rgb * (t.y * t.z) + k[6].rgb * (3.0 * t.z * t.z - 1.0)
//E         + k[7].rgb * (t.x * t.z) + k[8].rgb * (t.x * t.x - t.y * t.y)) * frame.environment.x;
    return light;
}
// Light `i`'s light reaching a surface at `at` facing `n`, unshadowed, and
// the way to it.
fn lamp(i: u32, at: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let point = lights[i];
    let to = point[0].xyz - at;
    let distance = length(to);
    let l = to / max(distance, 1e-6);
    let n_l = dot(n, l);
    if n_l <= 0.0 || distance >= point[0].w {
        return vec4(0.0);
    }
    let falloff = pow(clamp(1.0 - pow(distance / point[0].w, 4.0), 0.0, 1.0), 2.0) / max(distance * distance, 1e-4);
    let cone = clamp(dot(point[2].xyz, -l) + point[2].w, 0.0, 1.0);
    return vec4(point[1].rgb * point[1].w * falloff * cone * cone * n_l, distance);
}
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// The point and spot lights' light over pi at a hit: each shadowed by a
// ray of its own where there are eight or fewer; where more, one picked
// in proportion to its light reaching the hit, shadowed, and weighed by
// its odds (the same on average, a ray a hit).
fn lamps(at: vec3<f32>, n: vec3<f32>, lifted: vec3<f32>, ray: u32) -> vec3<f32> {
    var light = vec3(0.0);
    if trace.lights <= 8u {
        for (var i = 0u; i < trace.lights; i++) {
            let reaching = lamp(i, at, n);
            if any(reaching.rgb > vec3(0.0)) {
                let l = normalize(lights[i][0].xyz - at);
                if !blocked(lifted, l, reaching.w - trace.lift) {
                    light += reaching.rgb;
                }
            }
        }
        return light / PI;
    }
    var total = 0.0;
    for (var i = 0u; i < trace.lights; i++) {
        total += luminance(lamp(i, at, n).rgb);
    }
    if total <= 0.0 {
        return vec3(0.0);
    }
    var pick = f32(hash(ray * 9781u + trace.seed * 6271u) >> 8u) / 16777216.0 * total;
    for (var i = 0u; i < trace.lights; i++) {
        let reaching = lamp(i, at, n);
        let weight = luminance(reaching.rgb);
        if weight <= 0.0 {
            continue;
        }
        pick -= weight;
        if pick <= 0.0 || i + 1u == trace.lights {
            let l = normalize(lights[i][0].xyz - at);
            if blocked(lifted, l, reaching.w - trace.lift) {
                return vec3(0.0);
            }
            return reaching.rgb * (total / weight) / PI;
        }
    }
    return vec3(0.0);
}
fn hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}
fn vertex_at(i: u32) -> vec3<f32> {
    return vec3(vertices[i * 8u], vertices[i * 8u + 1u], vertices[i * 8u + 2u]);
}
fn normal_at(i: u32) -> vec3<f32> {
    return vec3(vertices[i * 8u + 3u], vertices[i * 8u + 4u], vertices[i * 8u + 5u]);
}

@compute @workgroup_size(64)
fn trace_rays(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= trace.count * trace.rays {
        return;
    }
    let ray = id.x % trace.rays;
    let probe = (trace.first + id.x / trace.rays) % trace.total;
    let origin = probe_place(probe);
    let d = ray_direction(ray);
    var q: ray_query;
    rayQueryInitialize(&q, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xffu, 0.0, 1e4, origin, d));
    rayQueryProceed(&q);
    let hit = rayQueryGetCommittedIntersection(&q);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        rays[id.x] = vec4(sky_along(d), trace.far);
        return;
    }
    if !hit.front_face {
        // Inside a mesh, or behind a wall: dark, and nearer than it is, so
        // the probe isn't trusted that way.
        rays[id.x] = vec4(0.0, 0.0, 0.0, -hit.t * 0.2);
        return;
    }
    let instance = instances[hit.instance_custom_data];
    let first = instance.geometry.y + hit.primitive_index * 3u;
    let base = instance.geometry.x;
    let i0 = base + indices[first];
    let i1 = base + indices[first + 1u];
    let i2 = base + indices[first + 2u];
    let b = vec3(1.0 - hit.barycentrics.x - hit.barycentrics.y, hit.barycentrics);
    let local = normal_at(i0) * b.x + normal_at(i1) * b.y + normal_at(i2) * b.z;
    // Normals turn by the inverse transpose: rows of world-to-object.
    let w = hit.world_to_object;
    var n = normalize(vec3(dot(w[0], local), dot(w[1], local), dot(w[2], local)));
    if dot(n, d) > 0.0 {
        n = -n;
    }
    let at = origin + d * hit.t;
    let lifted = at + n * trace.lift;
    var light = vec3(0.0);
    let sun = -frame.sun_direction.xyz;
    let facing = dot(n, sun);
    if facing > 0.0 && any(frame.sun_color.rgb > vec3(0.0)) && !blocked(lifted, sun, 1e4) {
        light += frame.sun_color.rgb * facing / PI;
    }
    light += lamps(at, n, lifted, id.x);
    // Light bounced before: the probes', the world's where none reach.
    let probed = probe_light(at, n, -d);
    light += mix(around(n), probed.rgb, probed.a);
    let albedo = instance.albedo.rgb * (1.0 - instance.albedo.w);
    rays[id.x] = vec4(albedo * light + instance.emissive.rgb, min(hit.t, trace.far));
}
