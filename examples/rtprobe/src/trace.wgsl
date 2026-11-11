enable wgpu_ray_query;

struct Params {
    size: vec2u,
    // Pinhole camera: origin and a basis.
    eye: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
}

@group(0) @binding(0) var tlas: acceleration_structure;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

fn primary(pixel: vec2f) -> RayDesc {
    let ndc = (pixel + 0.5) / vec2f(params.size) * 2.0 - 1.0;
    let dir = normalize(params.forward.xyz + ndc.x * params.right.xyz - ndc.y * params.up.xyz);
    return RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xffu, 0.001, 1000.0, params.eye.xyz, dir);
}

// Encodes a hit as (custom data + 1) << 16 | t * 256, front face in bit 15; 0 for a miss.
fn trace(ray: RayDesc) -> u32 {
    var rq: ray_query;
    rayQueryInitialize(&rq, tlas, ray);
    rayQueryProceed(&rq);
    let hit = rayQueryGetCommittedIntersection(&rq);
    if hit.kind == RAY_QUERY_INTERSECTION_NONE {
        return 0u;
    }
    let front = select(0u, 1u << 15u, hit.front_face);
    return ((hit.instance_custom_data + 1u) << 16u) | front | min(u32(hit.t * 256.0), 0x7fffu);
}

@compute @workgroup_size(8, 8)
fn cs_trace(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= params.size) {
        return;
    }
    out[id.y * params.size.x + id.x] = trace(primary(vec2f(id.xy)));
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_trace(@builtin(position) p: vec4f) -> @location(0) vec4u {
    let v = trace(primary(p.xy - 0.5));
    return vec4u(v, 0u, 0u, 1u);
}
