// Temporal antialiasing: what the meshes drew this frame (a fraction of a
// pixel aside) blended with the frames before, moved to where they are now
// by the depth and last frame's view (exact for the camera's motion over
// still meshes), the history clamped to this frame's neighbourhood so what
// it no longer holds doesn't trail.
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv);
}

struct Taa {
    // This frame's view and projection undone (unjittered), and last
    // frame's (unjittered).
    back: mat4x4<f32>,
    before: mat4x4<f32>,
    // The share of this frame (the rest the history's), 1 where the
    // history holds a frame to blend, the exposure, and the tone mapping
    // (0 neutral, 1 ACES, 2 none).
    params: vec4<f32>,
}
@group(0) @binding(0) var fresh: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;
@group(0) @binding(4) var<uniform> taa: Taa;
// Last frame's depth, kept beside the history.
@group(0) @binding(5) var depth_before: texture_2d<f32>;

struct Blended {
    @location(0) scene: vec4<f32>,
    @location(1) history: vec4<f32>,
    @location(2) depth: f32,
}

// A pixel with a part not finite as nothing (as the post chain reads it).
fn sane(c: vec4<f32>) -> vec4<f32> {
    let bad = (bitcast<vec4<u32>>(c) & vec4(0x7f800000u)) == vec4(0x7f800000u);
    return select(c, vec4(0.0), any(bad));
}
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// The tone curve on grey and its inverse (as the resolve has them): the
// blend is in the space the display will show, as a pixel's samples are.
fn curve(l: f32) -> f32 {
    if taa.params.w > 1.5 {
        return select(1.0 - 0.25 / l, l, l <= 0.5);
    }
    if taa.params.w > 0.5 {
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
    if taa.params.w > 1.5 {
        return select(0.25 / (1.0 - t), t, t <= 0.5);
    }
    if taa.params.w > 0.5 {
        let qa = 1.0 - t * 0.983729;
        let qb = 0.0245786 - t * 0.4329510;
        let qc = -(0.000090537 + t * 0.238081);
        return 0.6 * (-qb + sqrt(qb * qb - 4.0 * qa * qc)) / (2.0 * qa);
    }
    let c = select(t, 0.0576 / (1.0 - t) - 0.24 + 0.76, t >= 0.76);
    return select((1.0 - sqrt(max(1.0 - 25.0 * c, 0.0))) / 12.5, c + 0.04, c >= 0.04);
}
// A pixel (premultiplied) into the curve's space and back.
fn shown(c: vec4<f32>) -> vec4<f32> {
    let a = max(c.a, 1e-4);
    let x = c.rgb / a * taa.params.z;
    let l = max(luma(x), 1e-5);
    return vec4(x * (curve(l) / l) * c.a, c.a);
}
fn unshown(c: vec4<f32>) -> vec4<f32> {
    let a = max(c.a, 1e-4);
    let y = c.rgb / a;
    let l = max(luma(y), 1e-5);
    return vec4(y * (uncurve(l) / l) / taa.params.z * c.a, c.a);
}
fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}
fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// The history at `uv`, sharper than bilinear: Catmull-Rom by five bilinear
// reads (Jimenez's).
fn sharp(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(history));
    let at = uv * size;
    let centre = floor(at - 0.5) + 0.5;
    let f = at - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let at0 = (centre - 1.0) / size;
    let at3 = (centre + 2.0) / size;
    let at12 = (centre + w2 / w12) / size;
    var c = vec4(0.0);
    c += sane(textureSampleLevel(history, linear, vec2(at12.x, at0.y), 0.0)) * w12.x * w0.y;
    c += sane(textureSampleLevel(history, linear, vec2(at0.x, at12.y), 0.0)) * w0.x * w12.y;
    c += sane(textureSampleLevel(history, linear, at12, 0.0)) * w12.x * w12.y;
    c += sane(textureSampleLevel(history, linear, vec2(at3.x, at12.y), 0.0)) * w3.x * w12.y;
    c += sane(textureSampleLevel(history, linear, vec2(at12.x, at3.y), 0.0)) * w12.x * w3.y;
    let weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return max(c / weight, vec4(0.0));
}

@fragment fn blend(in: Out) -> Blended {
    let p = vec2<i32>(in.position.xy);
    let size = vec2<i32>(textureDimensions(fresh));
    let current = sane(textureLoad(fresh, p, 0));
    // The neighbourhood's mean and spread (in YCoCg), its alpha's bounds,
    // and its nearest depth (what an edge is reprojected by: no trail of
    // the background around a mesh).
    var sum = vec3(0.0);
    var squares = vec3(0.0);
    var lowest = 1.0;
    var highest = 0.0;
    var nearest = 1.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let q = clamp(p + vec2(dx, dy), vec2(0), size - 1);
            let c = sane(textureLoad(fresh, q, 0));
            let y = to_ycocg(c.rgb);
            sum += y;
            squares += y * y;
            lowest = min(lowest, c.a);
            highest = max(highest, c.a);
            nearest = min(nearest, textureLoad(depth, q, 0));
        }
    }
    var out = current;
    if taa.params.y > 0.5 {
        // Where this pixel's point was last frame.
        let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size);
        let ndc = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, nearest, 1.0);
        let world = taa.back * ndc;
        let then = taa.before * (world / world.w);
        let was = vec2(then.x / then.w * 0.5 + 0.5, 0.5 - then.y / then.w * 0.5);
        // Where the depth last frame held there isn't what this point's
        // would have been, something moved there (a mesh moving, or what
        // it uncovered): this frame alone, no trail.
        // (Any of the 3 by 3 there: the jitter moves an edge's depth a
        // pixel either way from frame to frame.)
        let expected = then.z / then.w;
        let slack = 2e-3 * max(1.0 - expected, 1e-3) + 1e-5;
        let there = vec2<i32>(was * vec2<f32>(size));
        var moved = true;
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let q = clamp(there + vec2(dx, dy), vec2(0), size - 1);
                let held = textureLoad(depth_before, q, 0).r;
                moved = moved && abs(held - expected) > slack;
            }
        }
        if all(was >= vec2(0.0)) && all(was <= vec2(1.0)) && then.w > 0.0 && !moved {
            var h = sharp(was);
            // Clamped toward the mean into the neighbourhood's spread
            // (a box of one deviation), so what it no longer holds goes.
            let mean = sum / 9.0;
            let deviation = sqrt(max(squares / 9.0 - mean * mean, vec3(0.0)));
            let hy = to_ycocg(h.rgb);
            let offset = hy - mean;
            let reach = max(abs(offset) / max(deviation, vec3(1e-4)), vec3(1.0));
            let clipped = mean + offset / max(reach.x, max(reach.y, reach.z));
            h = vec4(from_ycocg(clipped), clamp(h.a, lowest, highest));
            // In the space the display shows: a bright flicker counts as
            // it will show, and an edge averages as its samples would.
            out = unshown(mix(shown(h), shown(current), taa.params.x));
        }
    }
    return Blended(out, out, textureLoad(depth, p, 0));
}
