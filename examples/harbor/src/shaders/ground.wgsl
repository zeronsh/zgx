#zgx material 1
// The quay's ground: concrete slabs on the apron, asphalt in the yard,
// painted markings over both, and the wet as one gradient.
//
// - Concrete: expansion joints every 6 m sealed with tar; cracks from the
//   slab map, some sealed; each slab's tone; aggregate grain as a normal.
// - Asphalt: dark, its aggregate showing where it's worn, tyre rubber in
//   the wheel tracks along the lanes, oil where trucks wait.
// - Markings: the quay edge's yellow line, the apron's lanes, the yard's
//   slot lines and block outlines, bay numbers; drawn analytically and
//   anti-aliased by the pixel's footprint (`fwidth`), chipped and worn.
// - Wet: a height mask says where water stands; from it the ground goes
//   from standing water through soaked and damp to dry, darkening as its
//   pores fill and its roughness ramping dry ~0.72, damp 0.36-0.56, soaked
//   0.1-0.2, water 0.055; the grain's normal breaks up the mirror outside
//   the water; rain rings the water.
//
// Parameters:
//   mask    over the ground: height (r), damp (g), oil (b), wear (a)
//   slab    one 6 m slab, tiling: cracks (r), tar (g), grain (b), tone (a)
//   digits  0 to 9 side by side, white on black (r)
//   extent  the ground: x0, z0, width, depth (m)
//   yard    block pitch x, block pitch z, first block's z, the apron's end
//   wet     0 dry to 1 soaked, water standing in the dips
//   rain    0 to 1: raining now (rings)

const BAY: f32 = 12.892;
const ROW: f32 = 2.758;
const BAYS: f32 = 8.0;
const ROWS: f32 = 6.0;

// A stripe `half` wide about `d` = 0, anti-aliased by its footprint.
fn stripe(d: f32, half: f32) -> f32 {
    let w = max(fwidth(d), 1e-4);
    return 1.0 - smoothstep(half - w, half + w, abs(d));
}

// The distance (m) from `v` to the nearest multiple of `step`, signed.
fn to_grid(v: f32, step: f32) -> f32 {
    return (fract(v / step + 0.5) - 0.5) * step;
}

fn grain(p: vec2<f32>, scale: f32) -> f32 {
    return textureSample(params.slab, material_sampler, p / scale).b;
}

fn ripples(p: vec2<f32>, t: f32, scale: f32) -> vec2<f32> {
    let q = p * scale + vec2(t * 0.6, t * 0.35);
    let e = 0.05;
    let a = noise2(q);
    return vec2(noise2(q + vec2(e, 0.0)) - a, noise2(q + vec2(0.0, e)) - a) / e;
}

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let p = s.world_position.xz;
    let footprint = max(length(dpdx(p)), length(dpdy(p)));
    let mask = textureSample(params.mask, material_sampler, (p - params.extent.xy) / params.extent.zw);
    let apron = params.yard.w;
    let concrete = 1.0 - step(apron, p.y);

    // The base: concrete or asphalt.
    let slab = textureSample(params.slab, material_sampler, p / 6.0);
    let fine = grain(p, 1.7);
    var albedo = vec3(0.0);
    var rough = 0.72;
    // Concrete: its slab's tone, cracks, joints sealed with tar.
    var c = vec3(0.40, 0.39, 0.37) * (0.86 + 0.28 * slab.a) * (0.92 + 0.16 * slab.b);
    c *= 1.0 - 0.55 * slab.r * (1.0 - slab.g);
    let joint = max(stripe(to_grid(p.x, 6.0), 0.012), stripe(to_grid(p.y, 6.0), 0.012));
    let tar = max(slab.g * slab.r, joint);
    c = mix(c, vec3(0.025), tar);
    // Asphalt: dark, the aggregate showing where worn.
    var a = vec3(0.07, 0.07, 0.072) * (0.85 + 0.3 * fine);
    a = mix(a, vec3(0.15, 0.145, 0.14) * (0.8 + 0.4 * fine), mask.a * 0.7);
    albedo = mix(a, c, concrete);
    rough = mix(0.78, 0.72, concrete);
    rough = mix(rough, 0.4, tar * concrete);

    // Tyre rubber in the wheel tracks along the lanes (z).
    var lane = 0.0;
    if p.y < apron {
        lane = min(abs(p.y - 41.0), abs(p.y - 52.0));
    } else {
        let first = params.yard.z - params.yard.y * 0.5;
        lane = abs(to_grid(p.y - first, params.yard.y));
    }
    let track = exp(-pow((lane - 1.1) / 0.45, 2.0)) * (0.6 + 0.4 * noise2(p * vec2(0.05, 0.6)));
    albedo *= 1.0 - 0.45 * track;
    rough = mix(rough, 0.55, track * 0.6);
    // Oil: darker and smoother, a faint sheen.
    albedo *= 1.0 - 0.6 * mask.b;
    rough = mix(rough, 0.32, mask.b);

    // Markings.
    var paint = 0.0;
    var paint_color = vec3(0.75, 0.74, 0.70);
    // The quay edge's yellow line, the apron's dashed and solid lines.
    let edge = stripe(p.y - 1.7, 0.15);
    paint = max(paint, edge);
    paint = max(paint, stripe(p.y - 46.5, 0.08) * step(0.5, fract(p.x / 9.0)));
    paint = max(paint, stripe(p.y - (apron - 1.0), 0.1));
    // The yard's blocks: slot lines, a yellow outline, bay numbers.
    let bx = params.yard.x;
    let bz = params.yard.y;
    let cx = round(p.x / bx) * bx;
    let k = round((p.y - params.yard.z) / bz);
    let cz = params.yard.z + k * bz;
    let local = vec2(p.x - cx, p.y - cz);
    let half = vec2(BAYS * BAY * 0.5, ROWS * ROW * 0.5);
    let inside = step(abs(local.x), half.x + 0.2) * step(abs(local.y), half.y + 0.2)
        * step(apron + 2.0, p.y);
    let slots = max(stripe(to_grid(local.x + half.x, BAY), 0.05), stripe(to_grid(local.y + half.y, ROW), 0.05));
    paint = max(paint, slots * inside);
    let outline = max(
        stripe(abs(local.x) - half.x, 0.1) * step(abs(local.y), half.y + 0.1),
        stripe(abs(local.y) - half.y, 0.1) * step(abs(local.x), half.x + 0.1),
    ) * step(apron + 2.0, p.y);
    let yellow = max(edge, outline);
    paint = max(paint, outline);
    // Bay numbers: two digits in the lane before each bay, a metre wide
    // and one and a half tall: the block's row and the bay.
    let bay = clamp(floor((local.x + half.x) / BAY), 0.0, BAYS - 1.0);
    let at = vec2(local.x + half.x - (bay + 0.5) * BAY, local.y + half.y + 2.4);
    let glyph = vec2(at.x / 1.0 + 1.0, at.y / 1.5 + 0.5);
    let number = vec2(clamp(k + 1.0, 0.0, 9.0), bay + 1.0);
    let which = select(number.y, number.x, glyph.x < 1.0);
    let cell = vec2(fract(glyph.x), 1.0 - clamp(glyph.y, 0.0, 1.0));
    let digit = textureSample(params.digits, material_sampler, vec2((which + cell.x) / 10.0, cell.y)).r;
    let in_number = step(0.0, glyph.x) * step(glyph.x, 2.0) * step(0.0, glyph.y) * step(glyph.y, 1.0)
        * step(apron + 2.0, p.y) * step(abs(local.x), half.x);
    paint = max(paint, digit * in_number);
    // Worn and chipped: the paint lets the ground through in patches.
    let chips = smoothstep(0.35, 0.6, noise2(p * 1.7) * 0.6 + noise2(p * 7.0) * 0.4 + 0.15 - mask.a * 0.3);
    paint *= chips;
    paint_color = mix(paint_color, vec3(0.62, 0.48, 0.08), yellow);
    albedo = mix(albedo, paint_color, paint);
    rough = mix(rough, 0.5, paint);

    // The wet: one gradient from the height mask.
    let level = params.wet * 0.32;
    var w = clamp(1.0 - (mask.r - level) / 0.28, 0.0, 1.0) * min(params.wet * 4.0, 1.0);
    w = max(w, params.wet * (0.35 + 0.2 * mask.g));
    // Paint takes in less water than concrete and asphalt.
    let porous = 1.0 - 0.6 * paint;
    albedo *= mix(1.0, 0.5, smoothstep(0.2, 0.8, w) * porous);
    let standing = smoothstep(0.88, 0.95, w);
    albedo *= mix(1.0, 0.45, standing);
    rough = mix(rough, mix(0.56, 0.36, smoothstep(0.35, 0.6, w)), smoothstep(0.2, 0.45, w));
    rough = mix(rough, mix(0.2, 0.1, smoothstep(0.6, 0.85, w)), smoothstep(0.55, 0.7, w));
    rough = mix(rough, 0.055, standing);

    // The grain's normal, out of the water, faded where a pixel can't hold
    // it; rings on the water while it rains.
    let e = 0.02;
    let scale = mix(1.7, 6.0, concrete);
    let h0 = grain(p, scale);
    let gx = grain(p + vec2(e, 0.0), scale) - h0;
    let gz = grain(p + vec2(0.0, e), scale) - h0;
    let strength = 0.6 * (1.0 - standing) * (1.0 - smoothstep(0.02, 0.15, footprint));
    var slope = vec2(gx, gz) / e * 0.02 * strength;
    let close = 1.0 - smoothstep(0.01, 0.05, footprint);
    slope += ripples(p, frame.time * 6.0, 6.0) * 0.03 * params.rain * standing * close;
    m.normal = normalize(vec3(-slope.x, 1.0, -slope.y));
    m.base_color = albedo;
    m.metallic = 0.0;
    m.roughness = clamp(rough + (1.0 - close) * standing * 0.05, 0.04, 1.0);
    return m;
}
