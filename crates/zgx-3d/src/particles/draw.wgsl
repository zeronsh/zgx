// An emitter's particles drawn (particles/mod.rs) with the meshes, in their
// pass (particles/draw.rs): each a quad facing the camera, or stretched
// along its motion, coloured and sized by its age, soft at its edges. Read
// from the particles' buffer as instances, so no vertex shader reads a
// storage buffer (tier B draws these too).
//
// The frame's own uniforms (`frame`, binding 0, put before this text) give
// the camera as the meshes are drawn and their look: exposed, mapped and
// encoded as they are, or left in linear light where the post chain follows.

// `DrawUniforms` in particles/gpu.rs.
struct Draw {
    // The camera's right and up, in the world.
    right: vec4<f32>,
    up: vec4<f32>,
    // 0 a billboard, 1 a streak; a streak's length in seconds of motion.
    shape: vec4<f32>,
    // Colour and size over life: at its start, a third, two thirds, end.
    colors: array<vec4<f32>, 4>,
    sizes: vec4<f32>,
    // A pixel's width and height in clip space, and the view depths it
    // fades in between from the camera.
    viewport: vec4<f32>,
}
@group(0) @binding(1) var<uniform> draw: Draw;

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
    var size = p.size * over_life(draw.sizes, t);
    let right = draw.right.xyz;
    let up = draw.up.xyz;
    var coverage = 1.0;
    // At least two pixels wide (a soft disc a pixel wide can fall between
    // pixels' centres and draw nothing): one thinner is drawn two pixels
    // wide and as much fainter, so a far drop fades rather than flickering
    // in and out or vanishing.
    let middle = frame.view_projection * vec4(p.position, 1.0);
    let aside = frame.view_projection * vec4(p.position + right * size, 1.0);
    if middle.w > 0.0 && aside.w > 0.0 {
        let across = (aside.xy / aside.w - middle.xy / middle.w) / draw.viewport.xy;
        let pixels = length(across);
        if pixels < 2.0 {
            coverage = pixels / 2.0;
            size = size * 2.0 / max(pixels, 1e-4);
        }
    }
    // Its offset from its middle along the camera's right and up.
    var offset: vec2<f32>;
    if draw.shape.x > 0.5 {
        let moving = vec2(dot(p.velocity, right), dot(p.velocity, up));
        let across = length(moving);
        let along = select(vec2(0.0, 1.0), moving / across, across > 1e-5);
        let side = vec2(-along.y, along.x);
        let stretch = across * draw.shape.y;
        offset = along * corner.y * (stretch + size) * 0.5 + side * corner.x * size * 0.5;
        // Its light spread along where it went in the shutter's time: a
        // long streak as faint as it is longer than the drop.
        coverage *= size / (size + stretch);
    } else {
        offset = corner * size * 0.5;
    }
    // Fading in from the camera: none drawn over the whole view.
    let depth = dot(p.position - frame.camera.xyz, cross(up, right));
    coverage *= smoothstep(draw.viewport.z, draw.viewport.w, depth);
    let at = p.position + right * offset.x + up * offset.y;
    out.position = frame.view_projection * vec4(at, 1.0);
    out.corner = corner;
    out.color = p.color * color_over_life(t);
    out.color.a *= coverage;
    return out;
}

// As the meshes' (render/shader/display.wgsl).
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
    // Exposed and mapped as the meshes are (with the post chain: neither,
    // and none encoded, its last pass doing all three).
    var rgb = max(in.color.rgb * frame.params.z, vec3(0.0));
    if frame.look.x > 1.5 {
        // None.
    } else if frame.look.x > 0.5 {
        rgb = aces(rgb);
    } else {
        rgb = neutral(rgb);
    }
    if frame.params.y > 0.5 {
        rgb = encode(clamp(rgb, vec3(0.0), vec3(1.0)));
    }
    return vec4(rgb * alpha, alpha);
}
