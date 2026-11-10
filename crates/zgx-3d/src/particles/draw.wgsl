// An emitter's particles drawn (particles/mod.rs): each a quad facing the
// camera, or stretched along its motion, coloured and sized by its age,
// soft at its edges. Read from the particles' buffer as instances, so no
// vertex shader reads a storage buffer (tier B draws these too).

// `DrawUniforms` in particles/gpu.rs.
struct Draw {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    // Tone mapping (0 neutral, 1 ACES, 2 none), exposure as a factor,
    // whether to encode sRGB here, pixels' height in clip space.
    look: vec4<f32>,
    // 0 a billboard, 1 a streak; a streak's length in seconds of motion.
    shape: vec4<f32>,
    // Colour and size over life: at its start, a third, two thirds, end.
    colors: array<vec4<f32>, 4>,
    sizes: vec4<f32>,
}
@group(0) @binding(0) var<uniform> draw: Draw;

struct Instance {
    @location(0) position: vec3<f32>,
    @location(1) age: f32,
    @location(2) velocity: vec3<f32>,
    @location(3) life: f32,
    @location(4) color: vec4<f32>,
    @location(5) size: f32,
}
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec4<f32>,
}

fn over_life(keys: vec4<f32>, t: f32) -> f32 {
    let x = t * 3.0;
    if x < 1.0 {
        return mix(keys.x, keys.y, x);
    }
    if x < 2.0 {
        return mix(keys.y, keys.z, x - 1.0);
    }
    return mix(keys.z, keys.w, min(x - 2.0, 1.0));
}
fn color_over_life(t: f32) -> vec4<f32> {
    let x = t * 3.0;
    if x < 1.0 {
        return mix(draw.colors[0], draw.colors[1], x);
    }
    if x < 2.0 {
        return mix(draw.colors[1], draw.colors[2], x - 1.0);
    }
    return mix(draw.colors[2], draw.colors[3], min(x - 2.0, 1.0));
}

@vertex
fn vs(@builtin(vertex_index) vertex: u32, p: Instance) -> Out {
    var out: Out;
    if !(p.age < p.life) {
        // Dead: behind the near plane, nothing drawn.
        out.position = vec4(0.0, 0.0, -2.0, 1.0);
        return out;
    }
    let t = clamp(p.age / p.life, 0.0, 1.0);
    let corner = vec2(f32(vertex & 1u) * 2.0 - 1.0, f32(vertex >> 1u) * 2.0 - 1.0);
    let size = p.size * over_life(draw.sizes, t);
    var at = draw.view * vec4(p.position, 1.0);
    if draw.shape.x > 0.5 {
        let moving = (draw.view * vec4(p.velocity, 0.0)).xyz;
        let across = length(moving.xy);
        let along = select(vec2(0.0, 1.0), moving.xy / across, across > 1e-5);
        let side = vec2(-along.y, along.x);
        let stretch = across * draw.shape.y;
        at = vec4(at.xy + along * corner.y * (stretch + size) * 0.5 + side * corner.x * size * 0.5, at.zw);
    } else {
        at = vec4(at.xy + corner * size * 0.5, at.zw);
    }
    out.position = draw.projection * at;
    out.corner = corner;
    out.color = p.color * color_over_life(t);
    return out;
}

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
fn encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    var edge: f32;
    if draw.shape.x > 0.5 {
        edge = (1.0 - smoothstep(0.4, 1.0, abs(in.corner.x))) * (1.0 - smoothstep(0.6, 1.0, abs(in.corner.y)));
    } else {
        edge = 1.0 - smoothstep(0.5, 1.0, length(in.corner));
    }
    let alpha = clamp(in.color.a * edge, 0.0, 1.0);
    var rgb = max(in.color.rgb * draw.look.y, vec3(0.0));
    if draw.look.x > 1.5 {
        rgb = clamp(rgb, vec3(0.0), vec3(1.0));
    } else if draw.look.x > 0.5 {
        rgb = aces(rgb);
    } else {
        rgb = clamp(neutral(rgb), vec3(0.0), vec3(1.0));
    }
    if draw.look.z > 0.5 {
        rgb = encode(rgb);
    }
    return vec4(rgb * alpha, alpha);
}
