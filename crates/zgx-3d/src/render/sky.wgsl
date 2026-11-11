// A world's sky (`World::set_sky`): drawn behind the meshes, a grid
// over the frame at the far plane, where no mesh is. Light scattered by the
// air (Rayleigh) and haze (Mie) toward the eye from the sun, a sun disc, a
// ground under the horizon and a deck of clouds, in the units the world's
// lights are in: the sun's colour times its intensity is what falls on a
// surface square to it, and the sky gives back that light as radiance.
// The frame's uniforms (group 0) come before this, then display.wgsl's
// tone curve and encoding.

struct Sky {
    // Air's and haze's scattering at sea level, per metre (rgb), and the
    // haze's (w).
    rayleigh: vec4<f32>,
    // x: haze's forward scattering (g); y: the sun's angular radius
    // (radians); z: the eye's height above the sea (metres); w: how much
    // the ground reflects (grey).
    shape: vec4<f32>,
    // The clouds: x how much of the sky they cover (0 none), y their deck's
    // height (metres), z their texture's size (metres), w how far they've
    // drifted (metres along `drift`).
    clouds: vec4<f32>,
    // Which way they drift (xz, unit), and its seed (z); w unused.
    drift: vec4<f32>,
}
@group(1) @binding(0) var<uniform> sky: Sky;

struct Out {
    @builtin(position) position: vec4<f32>,
    // The frame's point on the far plane, homogeneous: divided per pixel.
    @location(0) far: vec4<f32>,
    // The air's light toward the eye before its phase (air's, haze's) and
    // its transmittance: worked out at the grid's corners, between them
    // interpolated (they change slowly across the sky; what changes fast,
    // the phases, the sun, the ground's edge and the clouds, is per pixel).
    @location(1) air: vec3<f32>,
    @location(2) haze: vec3<f32>,
    @location(3) through: vec3<f32>,
}

// The grid of the frame the air is worked out on, columns by rows (rows
// closer: the light changes fastest up from the horizon). Its draw is
// COLUMNS * ROWS * 6 vertices (`render::sky`).
const COLUMNS: u32 = 32u;
const ROWS: u32 = 64u;

// `m`'s inverse (a camera's view projection always has one).
fn inverse4(m: mat4x4<f32>) -> mat4x4<f32> {
    let a = m[0];
    let b = m[1];
    let c = m[2];
    let d = m[3];
    let s0 = a.x * b.y - b.x * a.y;
    let s1 = a.x * b.z - b.x * a.z;
    let s2 = a.x * b.w - b.x * a.w;
    let s3 = a.y * b.z - b.y * a.z;
    let s4 = a.y * b.w - b.y * a.w;
    let s5 = a.z * b.w - b.z * a.w;
    let c5 = c.z * d.w - d.z * c.w;
    let c4 = c.y * d.w - d.y * c.w;
    let c3 = c.y * d.z - d.y * c.z;
    let c2 = c.x * d.w - d.x * c.w;
    let c1 = c.x * d.z - d.x * c.z;
    let c0 = c.x * d.y - d.x * c.y;
    let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    let k = 1.0 / det;
    return mat4x4<f32>(
        vec4(b.y * c5 - b.z * c4 + b.w * c3, -a.y * c5 + a.z * c4 - a.w * c3, d.y * s5 - d.z * s4 + d.w * s3, -c.y * s5 + c.z * s4 - c.w * s3) * k,
        vec4(-b.x * c5 + b.z * c2 - b.w * c1, a.x * c5 - a.z * c2 + a.w * c1, -d.x * s5 + d.z * s2 - d.w * s1, c.x * s5 - c.z * s2 + c.w * s1) * k,
        vec4(b.x * c4 - b.y * c2 + b.w * c0, -a.x * c4 + a.y * c2 - a.w * c0, d.x * s4 - d.y * s2 + d.w * s0, -c.x * s4 + c.y * s2 - c.w * s0) * k,
        vec4(-b.x * c3 + b.y * c1 - b.z * c0, a.x * c3 - a.y * c1 + a.z * c0, -d.x * s3 + d.y * s1 - d.z * s0, c.x * s3 - c.y * s1 + c.z * s0) * k,
    );
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    // Two triangles a cell of the grid over the frame, at the far plane
    // (depth 1: drawn only where nothing nearer was).
    let cell = i / 6u;
    let corner = array(vec2(0u, 0u), vec2(1u, 0u), vec2(0u, 1u), vec2(0u, 1u), vec2(1u, 0u), vec2(1u, 1u))[i % 6u];
    let at = vec2(cell % COLUMNS, cell / COLUMNS) + corner;
    let xy = vec2(f32(at.x) / f32(COLUMNS), f32(at.y) / f32(ROWS)) * 2.0 - 1.0;
    var out: Out;
    out.position = vec4(xy, 1.0, 1.0);
    out.far = inverse4(frame.view_projection) * vec4(xy, 1.0, 1.0);
    let eye = frame.camera.xyz;
    let d = normalize(out.far.xyz / out.far.w - eye);
    let sun = -normalize(frame.sun_direction.xyz);
    let air = scattered(origin(eye), d, sun);
    out.air = air[0];
    out.haze = air[1];
    out.through = air[2];
    return out;
}

const EARTH: f32 = 6360e3;
const TOP: f32 = 6420e3;
const AIR_HEIGHT: f32 = 8000.0;
const HAZE_HEIGHT: f32 = 1200.0;
const PI: f32 = 3.14159265;
// Ozone's absorption per metre at sea level's air density (its share of the
// air's density curve, simplified): the zenith's blue at dusk.
const OZONE: vec3<f32> = vec3(0.65e-6, 1.881e-6, 0.085e-6);

// Where a ray from `o` along `d` (unit) leaves the sphere of radius `r`
// about the Earth's centre, or -1 if it misses it.
fn leave(o: vec3<f32>, d: vec3<f32>, r: f32) -> f32 {
    let b = dot(o, d);
    let c = dot(o, o) - r * r;
    let h = b * b - c;
    if h < 0.0 {
        return -1.0;
    }
    return -b + sqrt(h);
}
// Where it meets the ground, or -1 if it doesn't (or only behind).
fn ground_at(o: vec3<f32>, d: vec3<f32>) -> f32 {
    let b = dot(o, d);
    let c = dot(o, o) - EARTH * EARTH;
    let h = b * b - c;
    if h < 0.0 {
        return -1.0;
    }
    let t = -b - sqrt(h);
    return select(-1.0, t, t > 0.0);
}
// The column of an exponential atmosphere of scale height `scale` from
// `r` from the Earth's centre toward a zenith angle of cosine `c`, in
// metres at sea level's density: Schüler's closed form of the Chapman
// function (GPU Pro 3), to space (the air above the top adds nothing seen).
fn column(r: f32, c: f32, scale: f32) -> f32 {
    let x = EARTH / scale;
    let h = (r - EARTH) / scale;
    let k = sqrt(x + h);
    if c >= 0.0 {
        return scale * k / (k * c + 1.0) * exp(-h);
    }
    let x0 = sqrt(1.0 - c * c) * (x + h);
    return scale * (2.0 * sqrt(x0) * exp(x - x0) - k / (1.0 - k * c) * exp(-h));
}
// Air's, haze's and ozone's optical depth from `o` toward `d` (unit) out
// of the air, the Earth not in the way.
fn depth_to_top(o: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let r = length(o);
    let c = dot(o, d) / r;
    let air = column(r, c, AIR_HEIGHT);
    return vec3(air, column(r, c, HAZE_HEIGHT), air);
}
// The optical depth of `depth` (metres of air at sea level's density,
// of haze at its, of air for its ozone), per channel, and the light it lets
// through.
fn optical(depth: vec3<f32>) -> vec3<f32> {
    return sky.rayleigh.rgb * depth.x + vec3(sky.rayleigh.w * 1.11) * depth.y + OZONE * depth.z;
}
fn extinction(depth: vec3<f32>) -> vec3<f32> {
    return exp(-optical(depth));
}
fn henyey_greenstein(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(1.0 + g2 - 2.0 * g * c, 1.5));
}

// The eye's place about the Earth's centre.
fn origin(eye: vec3<f32>) -> vec3<f32> {
    return vec3(0.0, EARTH + max(sky.shape.z + eye.y, 1.0), 0.0);
}

// The light the air and the haze send toward the eye at `o` from along
// `d`, the sun toward `sun`, before their phases and the sun's irradiance,
// and the air's transmittance back along it (for what lies behind: the
// sun's disc, the ground).
fn scattered(o: vec3<f32>, d: vec3<f32>, sun: vec3<f32>) -> array<vec3<f32>, 3> {
    var end = leave(o, d, TOP);
    let hit = ground_at(o, d);
    if hit > 0.0 {
        end = hit;
    }
    // Steps closer together near the eye, where the light reaching it
    // comes from, each integrated as light both scattered and dimmed
    // within it (so a long step neither over- nor under-counts).
    let steps = 16;
    var air = 0.0;
    var haze = 0.0;
    var light_air = vec3(0.0);
    var light_haze = vec3(0.0);
    var near = 0.0;
    for (var i = 1; i <= steps; i++) {
        let f = f32(i) / f32(steps);
        let far = end * f * f;
        let step = far - near;
        let p = o + d * (near + 0.5 * step);
        near = far;
        let h = length(p) - EARTH;
        let air_here = exp(-h / AIR_HEIGHT) * step;
        let haze_here = exp(-h / HAZE_HEIGHT) * step;
        let before = vec3(air, haze, air);
        air += air_here;
        haze += haze_here;
        // Behind the Earth from the sun: in its shadow.
        if ground_at(p, sun) > 0.0 {
            continue;
        }
        let within = optical(vec3(air_here, haze_here, air_here));
        let kept = select((1.0 - exp(-within)) / within, vec3(1.0), within < vec3(1e-5));
        let t = extinction(before + depth_to_top(p, sun)) * kept;
        light_air += t * air_here;
        light_haze += t * haze_here;
    }
    return array<vec3<f32>, 3>(
        light_air * sky.rayleigh.rgb,
        light_haze * sky.rayleigh.w,
        extinction(vec3(air, haze, air)),
    );
}

// Value noise and its sum over octaves, for the clouds.
fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2(1.0, 0.0));
    let c = hash2(i + vec2(0.0, 1.0));
    let d = hash2(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p0: vec2<f32>, octaves: i32) -> f32 {
    var p = p0;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var i = 0; i < octaves; i++) {
        sum += noise(p) * amplitude;
        p = p * 2.03 + vec2(17.1, 9.7);
        amplitude *= 0.5;
    }
    return sum;
}
// How thick the deck is at `xz` (0 clear), as cover says, its detail to
// `octaves` (the looks toward the sun, shading it, need less).
fn cloud_density(xz: vec2<f32>, octaves: i32) -> f32 {
    let uv = (xz + sky.drift.xy * sky.clouds.w) / sky.clouds.z + sky.drift.z;
    let n = fbm(uv, octaves);
    let cover = sky.clouds.x;
    return smoothstep(1.0 - cover, 1.0 - cover + 0.35, n);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let eye = frame.camera.xyz;
    let d = normalize(in.far.xyz / in.far.w - eye);
    let sun = -normalize(frame.sun_direction.xyz);
    let e = frame.sun_color.rgb;
    let o = origin(eye);
    let c = dot(d, sun);
    let rayleigh_phase = 3.0 / (16.0 * PI) * (1.0 + c * c);
    let haze_phase = henyey_greenstein(c, sky.shape.x);
    let air = array(e * (in.air * rayleigh_phase + in.haze * haze_phase), in.through);
    var light = air[0];
    let hit = ground_at(o, d);
    if hit > 0.0 {
        // The ground, lit by the sun through the air and the sky above,
        // seen through the air between.
        let p = o + d * hit;
        let up = normalize(p);
        let sunlit = extinction(depth_to_top(p, sun)) * e * max(dot(up, sun), 0.0);
        let ambient = e * 0.08 * max(sun.y + 0.1, 0.0);
        light += (sunlit + ambient) * sky.shape.w / PI * air[1];
    } else {
        // The sun's disc: its irradiance over its solid angle, through the
        // air, limb darkened.
        let r = sky.shape.y;
        let cos_r = cos(r);
        if c > cos_r {
            let solid = 2.0 * PI * (1.0 - cos_r);
            let mu = sqrt(max(1.0 - pow(acos(min(c, 1.0)) / r, 2.0), 0.0));
            let limb = 1.0 - 0.6 * (1.0 - mu);
            light += e / solid * limb * air[1];
        }
        // The cloud deck, where the ray meets its height, faded toward the
        // horizon.
        if sky.clouds.x > 0.0 && d.y > 0.01 {
            let at = (sky.clouds.y - eye.y) / d.y;
            let xz = eye.xz + d.xz * at;
            let density = cloud_density(xz, 5);
            if density > 0.0 {
                // Light through the cloud toward the sun: thinner is
                // brighter (two looks along it), forward scattering lines
                // it silver toward the sun, bases darker.
                let toward = normalize(sun.xz + vec2(1e-4));
                let along = cloud_density(xz + toward * sky.clouds.z * 0.08, 3)
                    + cloud_density(xz + toward * sky.clouds.z * 0.2, 3);
                let through = exp(-along * 1.6);
                let silver = henyey_greenstein(c, 0.65) * 4.0 * PI;
                let deck = vec3(0.0, EARTH + sky.clouds.y, 0.0);
                let sunlit = extinction(depth_to_top(deck, sun)) * e;
                let base = mix(0.35, 1.0, through);
                let cloud = sunlit * (0.22 * base + 0.12 * through * silver) + air[0] * 0.6;
                // Seen through the air below the deck (an exponential
                // atmosphere, flat), and the air's light in front of it: the
                // share of the whole ray's that lies before it.
                let rise = max(sky.clouds.y - sky.shape.z - eye.y, 0.0);
                let below = vec3(
                    AIR_HEIGHT * (1.0 - exp(-rise / AIR_HEIGHT)),
                    HAZE_HEIGHT * (1.0 - exp(-rise / HAZE_HEIGHT)),
                    AIR_HEIGHT * (1.0 - exp(-rise / AIR_HEIGHT)),
                ) / d.y;
                let seen = extinction(below);
                let front = air[0] * clamp((1.0 - seen) / max(1.0 - air[1], vec3(1e-3)), vec3(0.0), vec3(1.0));
                let horizon = smoothstep(0.01, 0.15, d.y);
                light = mix(light, cloud * seen + front, density * horizon);
            }
        }
    }
    var lit = tone_map(light * frame.params.z);
    if frame.params.y > 0.5 {
        lit = encode(clamp(lit, vec3(0.0), vec3(1.0)));
    }
    return vec4(lit, 1.0);
}
