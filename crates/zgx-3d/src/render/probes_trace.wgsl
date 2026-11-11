// Probes updated by rays (`probes.rs`): this frame's probes each cast
// `trace.rays` rays (a spherical Fibonacci set, turned each round) into the
// scene's acceleration structure; each hit is lit by the sun and point
// lights (shadowed by rays of their own), by the probes' light from the
// last update (so light bounces on), and its own glow; a miss sees the
// sky. Each ray's light and distance go to `rays` for `probes_blend.wgsl`.
// Joined after the frame's declarations (`frame.wgsl`), the probes'
// (`shader/probes.wgsl`), their tags taken off, and the updaters' shared
// part (`probes_common.wgsl`).

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
// Whether anything lies between `from` and `distance` along `d`.
fn blocked(start: vec3<f32>, d: vec3<f32>, distance: f32) -> bool {
    var q: ray_query;
    rayQueryInitialize(&q, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xffu, 0.0, distance, start, d));
    rayQueryProceed(&q);
    return rayQueryGetCommittedIntersection(&q).kind != RAY_QUERY_INTERSECTION_NONE;
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
                if !blocked(lifted, toward(i, at, reaching.w), reaching.w - trace.lift) {
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
            if blocked(lifted, toward(i, at, reaching.w), reaching.w - trace.lift) {
                return vec3(0.0);
            }
            return reaching.rgb * (total / weight) / PI;
        }
    }
    return vec3(0.0);
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
        put(id.x, vec4(sky_along(d), trace.far));
        return;
    }
    if !hit.front_face {
        // Inside a mesh, or behind a wall: dark, and nearer than it is, so
        // the probe isn't trusted that way.
        put(id.x, vec4(0.0, 0.0, 0.0, -hit.t * 0.2));
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
    let turned = vec3(dot(w[0], local), dot(w[1], local), dot(w[2], local));
    // A geometry's normals may be nothing there (degenerate): facing the
    // ray, then.
    var n = select(-d, turned / max(length(turned), 1e-12), dot(turned, turned) > 1e-24);
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
    put(id.x, vec4(albedo * light + instance.emissive.rgb, min(hit.t, trace.far)));
}
