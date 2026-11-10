// The post chain: bloom's steps, and the last pass writing the target.
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv);
}

// A step of bloom from the one before (the scene for the first), or the one
// after it on the way back.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;

fn at(uv: vec2<f32>, x: f32, y: f32) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    return textureSampleLevel(source, linear, uv + vec2(x, y) * texel, 0.0).rgb;
}
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// 13 taps (Jimenez, "Next generation post processing in Call of Duty:
// Advanced Warfare"): five boxes of four, overlapping, the centre's weighted
// half. The first step weighs each box by 1 / (1 + its luma) (Karis's
// average), so a lone bright texel can't flicker into a large glow.
fn boxes(uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let a = at(uv, -2.0, 2.0);
    let b = at(uv, 0.0, 2.0);
    let c = at(uv, 2.0, 2.0);
    let d = at(uv, -2.0, 0.0);
    let e = at(uv, 0.0, 0.0);
    let f = at(uv, 2.0, 0.0);
    let g = at(uv, -2.0, -2.0);
    let h = at(uv, 0.0, -2.0);
    let i = at(uv, 2.0, -2.0);
    let j = at(uv, -1.0, 1.0);
    let k = at(uv, 1.0, 1.0);
    let l = at(uv, -1.0, -1.0);
    let m = at(uv, 1.0, -1.0);
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
@fragment fn first(in: Out) -> @location(0) vec4<f32> {
    return vec4(boxes(in.uv, true), 1.0);
}
@fragment fn down(in: Out) -> @location(0) vec4<f32> {
    return vec4(boxes(in.uv, false), 1.0);
}
// The step after, blurred by a 3 by 3 tent, added to this one (by the
// pipeline's blending).
@fragment fn up(in: Out) -> @location(0) vec4<f32> {
    let edges = at(in.uv, -1.0, 0.0) + at(in.uv, 1.0, 0.0) + at(in.uv, 0.0, -1.0) + at(in.uv, 0.0, 1.0);
    let corners = at(in.uv, -1.0, -1.0) + at(in.uv, 1.0, -1.0) + at(in.uv, -1.0, 1.0) + at(in.uv, 1.0, 1.0);
    return vec4((at(in.uv, 0.0, 0.0) * 4.0 + edges * 2.0 + corners) / 16.0, 1.0);
}

// The last pass.
struct Look {
    // The exposure's factor; the tone mapping (0 neutral, 1 ACES, 2 none);
    // whether to encode sRGB here.
    params: vec4<f32>,
    // What shows where nothing was drawn, premultiplied, as given.
    background: vec4<f32>,
    // Bloom's intensity, and 1 over its steps (its first holds their sum).
    bloom_mix: vec4<f32>,
}
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var bloom_sampler: sampler;
@group(0) @binding(3) var<uniform> look: Look;
override BLOOM: bool = false;

// The tone mappings, as the meshes' shader has them.
fn neutral(color: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desaturation = 0.15;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = color - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3(new_peak), g);
}
fn aces(color: vec3<f32>) -> vec3<f32> {
    let into = mat3x3(vec3(0.59719, 0.07600, 0.02840), vec3(0.35458, 0.90834, 0.13383), vec3(0.04823, 0.01566, 0.83777));
    let back = mat3x3(vec3(1.60475, -0.10208, -0.00327), vec3(-0.53108, 1.10813, -0.07276), vec3(-0.07367, -0.00605, 1.07602));
    let v = into * (color / 0.6);
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(back * (a / b), vec3(0.0), vec3(1.0));
}
fn tone_map(color: vec3<f32>) -> vec3<f32> {
    if look.params.y > 1.5 {
        return color;
    }
    if look.params.y > 0.5 {
        return aces(color);
    }
    return neutral(color);
}
fn encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}

@fragment fn last(in: Out) -> @location(0) vec4<f32> {
    let drawn = textureLoad(scene, vec2<i32>(in.position.xy), 0);
    var light = drawn.rgb;
    if BLOOM {
        let glow = textureSampleLevel(bloom, bloom_sampler, in.uv, 0.0).rgb * look.bloom_mix.y;
        light = mix(light, glow, look.bloom_mix.x);
    }
    // Mapped as the meshes' shader maps a pixel's colour (not its
    // premultiplied light), then multiplied by its coverage again.
    let alpha = drawn.a;
    let coverage = select(1.0, alpha, alpha > 1e-3);
    var mapped = clamp(tone_map(light / coverage * look.params.x), vec3(0.0), vec3(1.0));
    if look.params.z > 0.5 {
        mapped = encode(mapped);
    }
    return vec4(mapped * coverage, alpha) + look.background * (1.0 - alpha);
}
