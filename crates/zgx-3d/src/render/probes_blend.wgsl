// Probes' rays (`probes_trace.wgsl`) into their maps (`probes.rs`): each
// probe updated this frame draws its tile of the irradiance atlas (8 by 8
// texels) or of the depth atlas (16 by 16); each texel sums the probe's
// rays about its direction, cosine-weighted for light, sharply for
// distance. The pipeline blends the result over what was there by the
// round's weight (`set_blend_constant`).

struct Trace {
    turn: mat4x4<f32>,
    first: u32,
    count: u32,
    rays: u32,
    total: u32,
    far: f32,
    lift: f32,
    _pad: vec2<f32>,
}
@group(0) @binding(0) var<storage, read> rays: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> trace: Trace;
// The atlas's size in texels, and its tiles' side.
struct Atlas {
    size: vec2<f32>,
    tile: f32,
    // Distance (1) or light (0).
    depth: f32,
}
@group(0) @binding(2) var<uniform> atlas: Atlas;

const PI: f32 = 3.14159265;

fn fibonacci(i: u32, n: u32) -> vec3<f32> {
    let phi = 2.0 * PI * fract(f32(i) * 0.618034);
    let z = 1.0 - (2.0 * f32(i) + 1.0) / f32(n);
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}
// `octahedral`'s inverse (`shader/probes.wgsl`).
fn unfolded(o: vec2<f32>) -> vec3<f32> {
    var v = vec3(o, 1.0 - abs(o.x) - abs(o.y));
    if v.z < 0.0 {
        v = vec3((1.0 - abs(v.yx)) * select(vec2(-1.0), vec2(1.0), v.xy >= vec2(0.0)), v.z);
    }
    return normalize(v);
}

struct Tile {
    @builtin(position) position: vec4<f32>,
    // Which of this frame's probes.
    @location(0) @interpolate(flat) slot: u32,
    // Its tile's corner, in texels.
    @location(1) @interpolate(flat) corner: vec2<f32>,
}
@vertex fn tile(@builtin(vertex_index) vertex: u32, @builtin(instance_index) slot: u32) -> Tile {
    let probe = (trace.first + slot) % trace.total;
    let corner = vec2(f32(probe % 64u), f32(probe / 64u)) * atlas.tile;
    let at = corner + vec2(f32(vertex & 1u), f32((vertex >> 1u) & 1u)) * atlas.tile;
    let ndc = at / atlas.size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return Tile(vec4(ndc, 0.0, 1.0), slot, corner);
}

@fragment fn blend(in: Tile) -> @location(0) vec4<f32> {
    let n = atlas.tile;
    var x = floor(in.position.x - in.corner.x);
    var y = floor(in.position.y - in.corner.y);
    // A border texel repeats the one inside across the octahedron's edge,
    // so bilinear lookups wrap.
    let last = n - 1.0;
    let edge_x = x == 0.0 || x == last;
    let edge_y = y == 0.0 || y == last;
    if edge_x && edge_y {
        x = select(1.0, last - 1.0, x == 0.0);
        y = select(1.0, last - 1.0, y == 0.0);
    } else if edge_y {
        x = last - x;
        y = select(last - 1.0, 1.0, y == 0.0);
    } else if edge_x {
        y = last - y;
        x = select(last - 1.0, 1.0, x == 0.0);
    }
    let inner = n - 2.0;
    let d = unfolded((vec2(x, y) - 1.0 + 0.5) / inner * 2.0 - 1.0);
    let start = in.slot * trace.rays;
    var sum = vec3(0.0);
    var weights = 0.0;
    for (var i = 0u; i < trace.rays; i++) {
        let ray = rays[start + i];
        let along = normalize((trace.turn * vec4(fibonacci(i, trace.rays), 0.0)).xyz);
        let cosine = max(dot(d, along), 0.0);
        if atlas.depth > 0.5 {
            let w = pow(cosine, 50.0);
            let distance = min(abs(ray.w), trace.far);
            sum += vec3(distance, distance * distance, 0.0) * w;
            weights += w;
        } else {
            sum += ray.rgb * cosine;
            weights += cosine;
        }
    }
    return vec4(sum / max(weights, 1e-6), 1.0);
}
