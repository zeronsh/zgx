// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}
