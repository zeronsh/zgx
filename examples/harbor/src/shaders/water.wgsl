#zgx material 1
// The harbor's water. Its waves are a compute pass's (`waves.wgsl`): a
// field of heights and slopes over one 128 m tile, read here with
// `textureLoad` and blended between texels.
//
// Band-limited by each pixel's footprint: detail a pixel can't resolve
// (the field's short waves, the ripples) is averaged out of its normal
// and put into its roughness, which also starts from the wind's
// (Cox and Munk's slope variance for the waves the field doesn't hold).
// So the sea never aliases at any distance, and the sun's glint is a
// broad soft path rather than clipped patches.
//
// Murky: harbour water absorbs to a dark green-grey within a metre or
// two, so little of what's under it comes back; it reads by the sky it
// reflects. Refraction is a small bend, fading to none toward grazing
// angles and with distance: with no scene depth to reject what's in front
// of the surface, the worst case is no bend, never an edge of foreground.
//
// Parameters:
//   field     the wave field: height, slope x, slope z (a GpuTexture)
//   deep      the water body's colour (linear RGBA), its albedo
//   wind      the wind's speed over the water, m/s
//   rain      0 to 1: rain on it
//
// `frame.time` is read by the rings alone: the harbor makes this shader
// twice, once with them (raining, animated) and once without (calm, so a
// paused sea lets the world rest).

const SIDE: f32 = 512.0;
const TILE: f32 = 128.0;

fn texel(c: vec2<i32>) -> vec4<f32> {
    let n = i32(SIDE);
    return textureLoad(params.field, ((c % n) + n) % n, 0);
}

// The field at `p` (x, z): height and slope, blended between its texels.
fn field_at(p: vec2<f32>) -> vec3<f32> {
    let u = p / TILE * SIDE;
    let i = floor(u);
    let f = u - i;
    let c = vec2<i32>(i);
    let a = mix(texel(c), texel(c + vec2(1, 0)), f.x);
    let b = mix(texel(c + vec2(0, 1)), texel(c + vec2(1, 1)), f.x);
    return mix(a, b, f.y).xyz;
}

// The field averaged over a pixel's footprint `size` (m): taps across it,
// so waves shorter than the footprint average out rather than alias.
fn field_over(p: vec2<f32>, size: f32) -> vec3<f32> {
    if size < TILE / SIDE {
        return field_at(p);
    }
    let h = size * 0.35;
    return (field_at(p + vec2(h, h)) + field_at(p + vec2(-h, h)) + field_at(p + vec2(h, -h))
        + field_at(p + vec2(-h, -h))) * 0.25;
}

fn displace(v: VertexIn) -> Displaced {
    var d: Displaced;
    let w = field_at(v.world.xz);
    d.position = v.position + vec3(0.0, w.x, 0.0);
    d.normal = normalize(vec3(-w.y, 1.0, -w.z));
    return d;
}

// Small ripples: the slope of noise, drifting while time runs.
fn ripples(p: vec2<f32>, t: f32, scale: f32) -> vec2<f32> {
    let q = p * scale + vec2(t * 0.6, t * 0.35);
    let e = 0.05;
    let a = noise2(q);
    return vec2(noise2(q + vec2(e, 0.0)) - a, noise2(q + vec2(0.0, e)) - a) / e;
}

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let p = s.world_position.xz;
    // How many metres of water one pixel covers.
    let footprint = max(length(dpdx(p)), length(dpdy(p)));
    let field = field_over(p, footprint);
    // Ripples a metre across fade out as a pixel grows past a fifth of
    // them; rain's rings, a sixth of a metre, far sooner.
    let near = 1.0 - smoothstep(0.05, 0.25, footprint);
    let close = 1.0 - smoothstep(0.01, 0.04, footprint);
    // Past a few metres a pixel holds none of the field's slope: it fades
    // out, into roughness below.
    let resolved = 1.0 - smoothstep(0.5, 4.0, footprint);
    var slope = field.yz * resolved;
    slope += ripples(p, 0.0, 0.9) * 0.012 * near;
    slope += ripples(p, frame.time * 6.0, 6.0) * 0.03 * params.rain * close;
    let n = normalize(vec3(-slope.x, 1.0, -slope.y));
    m.normal = n;
    m.base_color = params.deep.rgb;
    m.metallic = 0.0;
    // Roughness from slope variance: the wind's waves shorter than the
    // field holds (Cox and Munk: 0.003 + 0.00512 U, about a third of it
    // under the field's half-metre), the ripples a pixel lost, and the
    // field's short waves its footprint averaged away; rain roughens.
    var variance = 0.35 * (0.003 + 0.00512 * params.wind);
    variance += (1.0 - near) * 0.0003;
    // The field's own slope variance (Cox and Munk's other two thirds), as
    // much of it as the pixel let go.
    variance += (1.0 - resolved) * 0.65 * (0.003 + 0.00512 * params.wind);
    variance += 0.004 * params.rain;
    // GGX alpha from variance, roughness its square root.
    m.roughness = clamp(sqrt(sqrt(2.0 * variance)), 0.05, 1.0);
    // Through the surface: a small bend, gone toward grazing and far off.
    let cos_v = clamp(dot(n, s.view), 0.0, 1.0);
    let reflected = 0.02 + 0.98 * pow(1.0 - cos_v, 5.0);
    let bend = 0.004 * smoothstep(0.15, 0.5, cos_v) * (1.0 - smoothstep(0.05, 0.4, footprint));
    let behind = scene_light(s.screen_uv + slope * bend, 0.0);
    // A metre or two of murk: dark green-grey, most of it gone.
    let murk = vec3(0.012, 0.022, 0.02);
    m.emissive += behind * murk * (1.0 - reflected);
    return m;
}
