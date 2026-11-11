#zgx material 1
// The harbor's water: a sum of six wind waves moving the surface (deep
// water's dispersion, so long waves outrun short ones), the same waves and
// a drift of small ripples (more while it rains) bending its normal, and
// the scene behind seen through it, absorbed as through a few metres of
// harbor water.
//
// Parameters:
//   deep      the water body's colour (linear RGBA), its albedo
//   wind      the wind's direction on the water (x, z) and speed (m/s)
//   chop      how high the waves stand, 1 as designed
//   rain      0 to 1: rain on it
//   unexpose  what undoes the frame's exposure on `scene_color`

const G: f32 = 9.81;
const WAVES: u32 = 6u;

// Wavelength (m) and height (m, crest to mean) of each, and its turn from
// the wind (radians).
fn wave(i: u32) -> vec3<f32> {
    switch i {
        case 0u: { return vec3(31.0, 0.16, 0.0); }
        case 1u: { return vec3(17.0, 0.09, 0.45); }
        case 2u: { return vec3(11.0, 0.06, -0.35); }
        case 3u: { return vec3(6.3, 0.035, 0.8); }
        case 4u: { return vec3(3.7, 0.02, -0.7); }
        default: { return vec3(2.1, 0.011, 0.25); }
    }
}

// The waves at `p` (x, z) at time `t`: height and its slope along x and z.
fn waves(p: vec2<f32>, t: f32) -> vec3<f32> {
    let w = normalize(params.wind.xy + vec2(1e-4, 0.0));
    var sum = vec3(0.0);
    for (var i = 0u; i < WAVES; i++) {
        let spec = wave(i);
        let c = cos(spec.z);
        let s = sin(spec.z);
        let dir = vec2(w.x * c - w.y * s, w.x * s + w.y * c);
        let k = 6.2831853 / spec.x;
        let omega = sqrt(G * k);
        let a = spec.y * params.chop;
        let phase = k * dot(dir, p) - omega * t + f32(i) * 1.7;
        sum += vec3(a * sin(phase), a * k * cos(phase) * dir.x, a * k * cos(phase) * dir.y);
    }
    return sum;
}

fn displace(v: VertexIn) -> Displaced {
    var d: Displaced;
    let h = waves(v.world.xz, frame.time);
    d.position = v.position + vec3(0.0, h.x, 0.0);
    d.normal = normalize(vec3(-h.y, 1.0, -h.z));
    return d;
}

// Small ripples: the slope of drifting noise, in two scales.
fn ripples(p: vec2<f32>, t: f32, scale: f32) -> vec2<f32> {
    let q = p * scale + vec2(t * 0.6, t * 0.35);
    let e = 0.05;
    let a = noise2(q);
    return vec2(noise2(q + vec2(e, 0.0)) - a, noise2(q + vec2(0.0, e)) - a) / e;
}

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let p = s.world_position.xz;
    let h = waves(p, frame.time);
    var slope = h.yz;
    slope += ripples(p, frame.time, 0.9) * 0.02;
    // Rain: a fine, fast stipple of rings' slopes.
    slope += ripples(p, frame.time * 6.0, 6.0) * 0.035 * params.rain;
    let n = normalize(vec3(-slope.x, 1.0, -slope.y));
    m.normal = n;
    m.base_color = params.deep.rgb;
    m.metallic = 0.0;
    m.roughness = 0.035 + 0.05 * params.rain;
    // Through the surface: what's behind, bent by the slope, absorbed by
    // a few metres of water (no depth to read yet), as much as isn't
    // reflected (Schlick, water's F0 of 0.02).
    let cos_v = clamp(dot(n, s.view), 0.0, 1.0);
    let reflected = 0.02 + 0.98 * pow(1.0 - cos_v, 5.0);
    let behind = scene_color(s.screen_uv + slope * 0.03, 0.0) * params.unexpose;
    let through = exp(-3.0 * vec3(0.75, 0.22, 0.28));
    m.emissive += behind * through * (1.0 - reflected);
    return m;
}
