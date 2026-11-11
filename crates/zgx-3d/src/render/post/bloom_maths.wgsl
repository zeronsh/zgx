// What bloom's two paths share (`post.wgsl`'s passes and `bloom.wgsl`'s
// dispatches): its taps, their weights and Karis's average. Prepended to
// both modules.

// A pixel with a part not finite (an infinite light's NaN) as nothing, so
// it can't spread through bloom or the exposure: by its bits, which fast
// maths can't fold away as it may `c != c`.
fn sane(c: vec4<f32>) -> vec4<f32> {
    let bad = (bitcast<vec4<u32>>(c) & vec4(0x7f800000u)) == vec4(0x7f800000u);
    return select(c, vec4(0.0), any(bad));
}
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// `t` at `uv`, `x` and `y` of its texels aside, filtered by `s`.
fn tap(t: texture_2d<f32>, s: sampler, uv: vec2<f32>, x: f32, y: f32) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t));
    return sane(textureSampleLevel(t, s, uv + vec2(x, y) * texel, 0.0)).rgb;
}
// 13 taps (Jimenez, "Next generation post processing in Call of Duty:
// Advanced Warfare"): five boxes of four, overlapping, the centre's weighted
// half. The first step weighs each box by 1 / (1 + its luma) (Karis's
// average), so a lone bright texel can't flicker into a large glow.
fn boxes(t: texture_2d<f32>, s: sampler, uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let a = tap(t, s, uv, -2.0, 2.0);
    let b = tap(t, s, uv, 0.0, 2.0);
    let c = tap(t, s, uv, 2.0, 2.0);
    let d = tap(t, s, uv, -2.0, 0.0);
    let e = tap(t, s, uv, 0.0, 0.0);
    let f = tap(t, s, uv, 2.0, 0.0);
    let g = tap(t, s, uv, -2.0, -2.0);
    let h = tap(t, s, uv, 0.0, -2.0);
    let i = tap(t, s, uv, 2.0, -2.0);
    let j = tap(t, s, uv, -1.0, 1.0);
    let k = tap(t, s, uv, 1.0, 1.0);
    let l = tap(t, s, uv, -1.0, -1.0);
    let m = tap(t, s, uv, 1.0, -1.0);
    let centre = (j + k + l + m) * 0.25;
    let corners = array((a + b + d + e) * 0.25, (b + c + e + f) * 0.25, (d + e + g + h) * 0.25, (e + f + h + i) * 0.25);
    var weights = vec4(0.125);
    var middle = 0.5;
    if karis {
        middle *= 1.0 / (1.0 + luma(centre));
        weights *= 1.0 / (1.0 + vec4(luma(corners[0]), luma(corners[1]), luma(corners[2]), luma(corners[3])));
    }
    let sum = centre * middle + corners[0] * weights.x + corners[1] * weights.y + corners[2] * weights.z + corners[3] * weights.w;
    return sum / (middle + weights.x + weights.y + weights.z + weights.w);
}
// The centre of the texel at `at` (its index and a half) of a step `size`
// large, as a place in it.
fn centre(at: vec2<f32>, size: vec2<u32>) -> vec2<f32> {
    return at / vec2<f32>(size);
}
// A step's texel as written: rounded to half floats here, so a render
// target's conversion and a storage write's have nothing left to round.
// Held first to the most every step format holds: a step up adds two
// steps, each up to the scene's most, and a sum past it would round to
// infinity, which the next step's taps read as nothing (`sane`), dark at a
// glow's centre. Packed floats hold less than half floats (65504): 65024 in
// their 11-bit red and green, 64512 in their 10-bit blue; the same on
// every path, so dispatches and passes draw alike.
const STEP_MOST = vec3(65024.0, 65024.0, 64512.0);
fn step_out(c: vec3<f32>) -> vec4<f32> {
    return vec4(quantizeToF16(min(c, STEP_MOST)), 1.0);
}
// `t` blurred by a 3 by 3 tent at `uv`.
fn tent(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec3<f32> {
    let edges = tap(t, s, uv, -1.0, 0.0) + tap(t, s, uv, 1.0, 0.0) + tap(t, s, uv, 0.0, -1.0) + tap(t, s, uv, 0.0, 1.0);
    let corners = tap(t, s, uv, -1.0, -1.0) + tap(t, s, uv, 1.0, -1.0) + tap(t, s, uv, -1.0, 1.0) + tap(t, s, uv, 1.0, 1.0);
    return (tap(t, s, uv, 0.0, 0.0) * 4.0 + edges * 2.0 + corners) / 16.0;
}
