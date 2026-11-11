// Probes updated without ray queries (`probes_raster.rs`). Each of this
// frame's probes draws the scene (its traced meshes as one list of
// triangles in the world, `Soup`) into six faces of `FACE` by `FACE`
// texels about it, a draw a face. Each fragment is lit as a ray's hit is
// in `probes_trace.wgsl`: the sun (shadowed by the frame's shadow map),
// every point and spot light (unshadowed, as the meshes drawn directly
// are), the probes' light from before, and its glow. It keeps its
// distance in alpha, negative behind a surface. Then `faces_to_rays`
// reads the faces along each probe's rays into `rays`, the sky where
// nothing was drawn, for the blend (`probes_blend.wgsl`). Joined after
// the frame's declarations, the probes' and the updaters' shared part.

const FACE: u32 = 16u;
const NEAR: f32 = 0.01;
const FAR: f32 = 10000.0;
// A face texel's alpha where nothing was drawn.
const EMPTY: f32 = 1e30;

@group(0) @binding(23) var faces: texture_2d<f32>;

struct Captured {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) glow: vec3<f32>,
    @location(4) @interpolate(flat) origin: vec3<f32>,
}

// `rel` (a point less the probe's place) on cube face `face`: across and
// down (each from -1 to 1 times the distance ahead), and ahead. As
// `environment.rs`'s `face_of` reads a direction.
fn on_face(face: u32, rel: vec3<f32>) -> vec3<f32> {
    switch face {
        case 0u: { return vec3(-rel.z, -rel.y, rel.x); }
        case 1u: { return vec3(rel.z, -rel.y, -rel.x); }
        case 2u: { return vec3(rel.x, rel.z, rel.y); }
        case 3u: { return vec3(rel.x, -rel.z, -rel.y); }
        case 4u: { return vec3(rel.x, -rel.y, rel.z); }
        default: { return vec3(-rel.x, -rel.y, -rel.z); }
    }
}

// `face` is the draw's instance: this frame's probe `face / 6`, its face
// `face % 6`.
@vertex fn capture_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) glow: vec3<f32>,
    @builtin(instance_index) face: u32,
) -> Captured {
    let origin = probe_place((trace.first + face / 6u) % trace.total);
    let f = on_face(face % 6u, position - origin);
    let clip = vec4(f.x, -f.y, (f.z - NEAR) * FAR / (FAR - NEAR), f.z);
    return Captured(clip, position, normal, albedo, glow, origin);
}

// How much of the sun reaches `at` (facing `n`): the frame's shadow map,
// as last drawn (the whole of it lit outside it, or with no shadows).
fn sun_reaching(at: vec3<f32>, n: vec3<f32>) -> f32 {
    if frame.params.w < 0.5 {
        return 1.0;
    }
    let p = frame.shadow * vec4(at + n * frame.shadow_texel.z, 1.0);
    let uv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || p.z > 1.0 {
        return 1.0;
    }
    return textureSampleCompareLevel(shadow_map, shadow_sampler, uv, p.z);
}

@fragment fn capture_fragment(in: Captured) -> @location(0) vec4<f32> {
    let out = in.world - in.origin;
    let distance = max(length(out), 1e-6);
    let along = out / distance;
    let turned = in.normal;
    var n = select(-along, turned / max(length(turned), 1e-12), dot(turned, turned) > 1e-24);
    // Seen from behind: inside a mesh, or behind a wall. Dark, and nearer
    // than it is, so the probe isn't trusted that way.
    if dot(n, along) > 0.0 {
        return vec4(0.0, 0.0, 0.0, -distance * 0.2);
    }
    var light = vec3(0.0);
    let sun = -frame.sun_direction.xyz;
    let facing = dot(n, sun);
    if facing > 0.0 {
        light += frame.sun_color.rgb * facing * sun_reaching(in.world, n) / PI;
    }
    for (var i = 0u; i < trace.lights; i++) {
        light += lamp(i, in.world, n).rgb / PI;
    }
    let probed = probe_light(in.world, n, -along);
    light += mix(around(n), probed.rgb, probed.a);
    return vec4(in.albedo * light + in.glow, min(distance, trace.far));
}

// The faces along each of this frame's probes' rays.
@compute @workgroup_size(64)
fn faces_to_rays(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= trace.count * trace.rays {
        return;
    }
    let slot = id.x / trace.rays;
    let d = ray_direction(id.x % trace.rays);
    let a = abs(d);
    var face = 0u;
    var uv = vec2(0.0);
    if a.x >= a.y && a.x >= a.z {
        face = select(1u, 0u, d.x > 0.0);
        uv = select(vec2(d.z, -d.y), vec2(-d.z, -d.y), d.x > 0.0) / a.x;
    } else if a.y >= a.z {
        face = select(3u, 2u, d.y > 0.0);
        uv = select(vec2(d.x, -d.z), vec2(d.x, d.z), d.y > 0.0) / a.y;
    } else {
        face = select(5u, 4u, d.z > 0.0);
        uv = select(vec2(-d.x, -d.y), vec2(d.x, -d.y), d.z > 0.0) / a.z;
    }
    let texel = min(vec2<u32>((uv * 0.5 + 0.5) * f32(FACE)), vec2(FACE - 1u));
    let seen = textureLoad(faces, vec2(face, slot) * FACE + texel, 0);
    if seen.a >= EMPTY * 0.5 {
        rays[id.x] = vec4(sky_along(d), trace.far);
        return;
    }
    rays[id.x] = seen;
}
