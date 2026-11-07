// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}
