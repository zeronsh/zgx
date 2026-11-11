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

// A step up's own step down.
@group(0) @binding(2) var base: texture_2d<f32>;
// Each step's texel centre, and what it writes, as `bloom.wgsl`'s dispatches
// have them (`centre`, `step_out`): a pixel's position is its texel's
// index and a half exactly, and a step down is half the one before.
@fragment fn first(in: Out) -> @location(0) vec4<f32> {
    let size = max(textureDimensions(source) / 2u, vec2(1u));
    return step_out(boxes(source, linear, centre(in.position.xy, size), true));
}
@fragment fn down(in: Out) -> @location(0) vec4<f32> {
    let size = max(textureDimensions(source) / 2u, vec2(1u));
    return step_out(boxes(source, linear, centre(in.position.xy, size), false));
}
// The step after, blurred by a 3 by 3 tent, added to this one's own down,
// into a chain of its own (no blending: the dispatches' sums, to the bit).
@fragment fn up(in: Out) -> @location(0) vec4<f32> {
    let own = textureLoad(base, vec2<i32>(in.position.xy), 0).rgb;
    let size = textureDimensions(base);
    return step_out(own + tent(source, linear, centre(in.position.xy, size)));
}

// The last pass.
struct Look {
    // The exposure's factor; the tone mapping (0 neutral, 1 ACES, 2 none);
    // whether to encode sRGB here.
    params: vec4<f32>,
    // What shows where nothing was drawn, premultiplied, as given.
    background: vec4<f32>,
    // Bloom's intensity, 1 over its steps (its first and the second's tent
    // hold their sum), and whether there is a second (1) or not (0).
    bloom_mix: vec4<f32>,
    // The vignette's, grain's and chromatic aberration's intensities, and
    // the frame's index (grain's seed).
    lens: vec4<f32>,
    // Grading: white balance's L, M, S factors and the saturation; lift and
    // the table's entries a side; 1 over gamma; gain.
    balance: vec4<f32>,
    lift: vec4<f32>,
    gamma: vec4<f32>,
    gain: vec4<f32>,
}
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var bloom_sampler: sampler;
@group(0) @binding(3) var<uniform> look: Look;
// Automatic exposure's state (`exposure.wgsl`'s), as the last frame left
// it; with it on (look.params.w), the scene's mean brought to middle grey.
struct Adapted {
    adapted: f32,
    target_: f32,
    made: f32,
    pad: f32,
}
@group(0) @binding(6) var<uniform> adapted: Adapted;
fn exposure() -> f32 {
    var e = look.params.x;
    if look.params.w > 0.5 && adapted.made > 0.5 {
        e *= exp2(log2(0.18) - adapted.adapted);
    }
    return e;
}
@group(0) @binding(4) var bloom_wider: texture_2d<f32>;
override BLOOM: bool = false;
override VIGNETTE: bool = false;
override GRAIN: bool = false;
override ABERRATION: bool = false;
override GRADE: bool = false;
override TABLE: bool = false;
@group(0) @binding(5) var table: texture_2d<f32>;

// Linear sRGB to LMS and back (CAT02, as Unity's white balance).
const TO_LMS = mat3x3(vec3(0.390405, 0.0708416, 0.0231082), vec3(0.549941, 0.963172, 0.128021), vec3(0.00892632, 0.00135775, 0.936245));
const FROM_LMS = mat3x3(vec3(2.85847, -0.210182, -0.041812), vec3(-1.62879, 1.15820, -0.118169), vec3(-0.024891, 0.000324281, 1.06867));
fn decode(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}
// Linear sRGB (Rec. 709, D65) to ACES's AP1 primaries (D60, Bradford), and
// ACEScct's log encoding of it (S-2016-001).
const TO_AP1 = mat3x3(vec3(0.61309732, 0.07019422, 0.02061560), vec3(0.33952285, 0.91635557, 0.10956983), vec3(0.04737928, 0.01345259, 0.86981512));
fn acescct(c: vec3<f32>) -> vec3<f32> {
    let low = 10.5402377416545 * c + 0.0729055341958355;
    let high = (log2(max(c, vec3(1e-10))) + 9.72) / 17.52;
    return clamp(select(high, low, c <= vec3(0.0078125)), vec3(0.0), vec3(1.0));
}
// Lift, gamma, gain and saturation on what the display shows (encoded).
fn graded(p: vec3<f32>) -> vec3<f32> {
    var c = look.gain.rgb * (p + look.lift.rgb * (1.0 - p));
    c = pow(max(c, vec3(0.0)), look.gamma.rgb);
    let l = luma(c);
    return clamp(l + (c - l) * look.balance.w, vec3(0.0), vec3(1.0));
}
// The table at `p` (encoded), between its entries trilinearly: two of its
// strip's N by N tables (blue's two nearest), each read bilinearly.
fn looked_up(p: vec3<f32>) -> vec3<f32> {
    let n = look.lift.w;
    let blue = p.b * (n - 1.0);
    let slice = min(floor(blue), n - 2.0);
    let x = (p.r * (n - 1.0) + 0.5 + slice * n) / (n * n);
    let y = (p.g * (n - 1.0) + 0.5) / n;
    let near = textureSampleLevel(table, bloom_sampler, vec2(x, y), 0.0).rgb;
    let far = textureSampleLevel(table, bloom_sampler, vec2(x + 1.0 / n, y), 0.0).rgb;
    return mix(near, far, blue - slice);
}

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

// A number from 0 to 1 for each pixel and frame (PCG's hash).
fn hash(x: u32, y: u32, frame: u32) -> f32 {
    var v = x * 1664525u + y * 1013904223u + frame * 2654435769u;
    v = v * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    v = (v >> 22u) ^ v;
    return f32(v) / 4294967295.0;
}


@fragment fn last(in: Out) -> @location(0) vec4<f32> {
    var drawn = sane(textureLoad(scene, vec2<i32>(in.position.xy), 0));
    if ABERRATION {
        // Red spread outward and blue inward, the more toward the edges, as
        // a lens refracts them apart: a hundredth of the frame's width at
        // its edge at intensity 1.
        let shift = (in.uv - 0.5) * look.lens.z * 0.02;
        let red = sane(textureSampleLevel(scene, bloom_sampler, in.uv - shift, 0.0));
        let blue = sane(textureSampleLevel(scene, bloom_sampler, in.uv + shift, 0.0));
        drawn = vec4(red.r, drawn.g, blue.b, max(drawn.a, max(red.a, blue.a)));
    }
    return finished(drawn, in, vec2<f32>(textureDimensions(scene)));
}

// The last pass where nothing before it needs the scene: the meshes' samples
// resolved here, as `resolve` resolves them, and finished (no aberration:
// it reads the scene around the pixel).
@fragment fn last_samples(in: Out) -> @location(0) vec4<f32> {
    let drawn = resolved(vec2<i32>(in.position.xy));
    return finished(drawn, in, vec2<f32>(textureDimensions(samples)));
}

// `drawn` (a pixel of the scene, premultiplied) exposed, bloom mixed in,
// graded, mapped and encoded, over the background, in a frame of `size`.
fn finished(drawn: vec4<f32>, in: Out, size: vec2<f32>) -> vec4<f32> {
    // From the centre, the longer side's half 1.
    let off = (in.uv - 0.5) * 2.0 * size / max(size.x, size.y);
    var light = drawn.rgb;
    if BLOOM {
        // The first step, with the second's tent added (bloom's last step
        // up, here rather than a pass of its own); a 1 by 1 stand-in where
        // there's one step adds nothing (bloom_mix.z 0).
        var glow = textureSampleLevel(bloom, bloom_sampler, in.uv, 0.0).rgb;
        glow += tent(bloom_wider, bloom_sampler, in.uv) * look.bloom_mix.z;
        glow *= look.bloom_mix.y;
        light = mix(light, glow, look.bloom_mix.x);
    }
    // Mapped as the meshes' shader maps a pixel's colour (not its
    // premultiplied light), then multiplied by its coverage again.
    let alpha = drawn.a;
    let coverage = select(1.0, alpha, alpha > 1e-3);
    var exposed = light / coverage * exposure();
    if GRADE {
        // White balance: the eye adapted to another white, in LMS.
        exposed = FROM_LMS * ((TO_LMS * exposed) * look.balance.xyz);
    }
    if VIGNETTE {
        // Darker toward the corners, as cos⁴ of the angle off the axis
        // falls: at intensity 1, the corners a quarter.
        let r2 = dot(off, off) * 0.5 * look.lens.x;
        exposed *= 1.0 / ((1.0 + r2) * (1.0 + r2));
    }
    var mapped = clamp(tone_map(exposed), vec3(0.0), vec3(1.0));
    if TABLE && look.gamma.w > 0.5 {
        // A table in ACEScct maps the light to what the display shows
        // itself, in place of the tone curve.
        var shown = looked_up(acescct(TO_AP1 * max(exposed, vec3(0.0))));
        if GRADE {
            shown = graded(shown);
        }
        mapped = decode(shown);
    } else if GRADE || TABLE {
        var shown = encode(mapped);
        if GRADE {
            shown = graded(shown);
        }
        if TABLE {
            shown = looked_up(shown);
        }
        mapped = decode(shown);
    }
    if GRAIN {
        // Noise of the frame's index, so a recorded frame repeats, most in
        // the middle tones, in a space near the eye's.
        let at = vec2<u32>(in.position.xy);
        let noise = hash(at.x, at.y, u32(look.lens.w)) - 0.5;
        var tone = sqrt(mapped);
        tone = clamp(tone + noise * look.lens.y * 4.0 * tone * (1.0 - tone), vec3(0.0), vec3(1.0));
        mapped = tone * tone;
    }
    if look.params.z > 0.5 {
        mapped = encode(mapped);
    }
    return vec4(mapped * coverage, alpha) + look.background * (1.0 - alpha);
}

// The active tone curve on grey (a luma), and its inverse: what a sample
// of that luma shows as, and what luma shows as that.
fn curve(l: f32) -> f32 {
    if look.params.y > 1.5 {
        // Linear clamps at 1, which can't be undone: as it up to 0.5, then
        // toward 1 (1 - 0.25 / l), so light past 1 survives the resolve.
        return select(1.0 - 0.25 / l, l, l <= 0.5);
    }
    if look.params.y > 0.5 {
        let v = l / 0.6;
        return clamp((v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.4329510) + 0.238081), 0.0, 1.0);
    }
    let c = select(l - 0.04, l - 6.25 * l * l, l < 0.08);
    if c < 0.76 {
        return c;
    }
    return 1.0 - 0.0576 / (c + 0.24 - 0.76);
}
fn uncurve(y: f32) -> f32 {
    let t = min(y, 0.9999);
    if look.params.y > 1.5 {
        return select(0.25 / (1.0 - t), t, t <= 0.5);
    }
    if look.params.y > 0.5 {
        // v² + a v - b = t (c v² + d v + e), for v.
        let qa = 1.0 - t * 0.983729;
        let qb = 0.0245786 - t * 0.4329510;
        let qc = -(0.000090537 + t * 0.238081);
        return 0.6 * (-qb + sqrt(qb * qb - 4.0 * qa * qc)) / (2.0 * qa);
    }
    let c = select(t, 0.0576 / (1.0 - t) - 0.24 + 0.76, t >= 0.76);
    return select((1.0 - sqrt(max(1.0 - 25.0 * c, 0.0))) / 12.5, c + 0.04, c >= 0.04);
}

// The meshes' samples averaged into the scene as the last pass will show
// them: each sample scaled by what the tone curve makes of its luma
// (exposed as the last pass will), averaged, and the curve undone. A grey
// edge against a highlight then averages as the path without the chain
// averages mapped samples, a coloured one closely (after Karis's
// 1 / (1 + luma), with the curve itself).
@group(0) @binding(0) var samples: texture_multisampled_2d<f32>;
@fragment fn resolve(in: Out) -> @location(0) vec4<f32> {
    return resolved(vec2<i32>(in.position.xy));
}
fn resolved(at: vec2<i32>) -> vec4<f32> {
    let n = textureNumSamples(samples);
    let exposed_by = exposure();
    var sum = vec4(0.0);
    for (var i = 0u; i < n; i++) {
        let c = sane(textureLoad(samples, at, i32(i)));
        // A sample's own colour (its coverage divided out, as the last pass
        // maps a pixel's), mapped, and its coverage again.
        let a = max(c.a, 1e-4);
        let x = c.rgb / a * exposed_by;
        let l = max(luma(x), 1e-5);
        sum += vec4(x * (curve(l) / l) * c.a, c.a);
    }
    let mean = sum / f32(n);
    let a = max(mean.a, 1e-4);
    let y = mean.rgb / a;
    let l = max(luma(y), 1e-5);
    return vec4(y * (uncurve(l) / l) / exposed_by * mean.a, mean.a);
}

// The scene's other half copied back into it (after an odd count of the
// app's HDR passes).
@fragment fn copy(in: Out) -> @location(0) vec4<f32> {
    return textureLoad(source, vec2<i32>(in.position.xy), 0);
}
