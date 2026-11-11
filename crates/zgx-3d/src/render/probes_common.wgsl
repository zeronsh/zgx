// What the probes' updaters share (`probes.rs`): the tracer's
// (`probes_trace.wgsl`) and the rasteriser's (`probes_raster.wgsl`). Joined
// after the frame's declarations (`frame.wgsl`) and the probes'
// (`shader/probes.wgsl`), their tags taken off.

struct Trace {
    // Turns this round's rays.
    turn: mat4x4<f32>,
    // The first probe updated (of all volumes', in order), how many, rays
    // a probe, and how many probes there are.
    first: u32,
    count: u32,
    rays: u32,
    total: u32,
    // The farthest a ray's distance counts, and how far off a surface a
    // shadow ray starts.
    far: f32,
    lift: f32,
    // Point and spot lights in `lights`; the round.
    lights: u32,
    seed: u32,
    // For tests (`World::poison_probe_rays`): 0, or every seventh ray
    // brings back no finite light (1), let into the probes (2); and a zero
    // to divide by.
    poison: u32,
    zero: f32,
}
@group(0) @binding(20) var<storage, read_write> rays: array<vec4<f32>>;
@group(0) @binding(21) var<uniform> trace: Trace;
// Every point and spot light (`trace_lights`): place and range, colour
// and intensity, cone; not the frame's `points`, which are the eight
// nearest the camera's target.
@group(0) @binding(22) var<storage, read> lights: array<array<vec4<f32>, 3>>;

const PI: f32 = 3.14159265;

// Ray `i`'s light and distance into `rays`; for tests, every seventh made
// infinite or not a number, from a zero only known as the GPU runs.
fn put(i: u32, value: vec4<f32>) {
    var out = value;
    if trace.poison != 0u && i % 7u == 3u {
        let infinite = trace.far / trace.zero;
        out = vec4(select(vec3(infinite), vec3(infinite * trace.zero), i % 14u == 3u) + value.rgb, value.w);
    }
    rays[i] = out;
}

// The `i`th of `n` directions spread evenly over the sphere.
fn fibonacci(i: u32, n: u32) -> vec3<f32> {
    let phi = 2.0 * PI * fract(f32(i) * 0.618034);
    let z = 1.0 - (2.0 * f32(i) + 1.0) / f32(n);
    let r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}
fn ray_direction(i: u32) -> vec3<f32> {
    return normalize((trace.turn * vec4(fibonacci(i, trace.rays), 0.0)).xyz);
}
// Probe `index`'s place (of all volumes', in order).
fn probe_place(index: u32) -> vec3<f32> {
    for (var i = 0u; i < probe_set.count.x; i++) {
        // A pointer, not a copy: DXC 1.9 at -Od (wgpu's debug builds)
        // crashes on a struct copied from a uniform array in a loop that
        // returns (`probes_hold_no_volume_by_value`).
        let volume = &probe_set.volumes[i];
        let first = u32(volume.origin.w);
        let n = volume.counts.x * volume.counts.y * volume.counts.z;
        if index >= first && index < first + n {
            let local = index - first;
            let at = vec3(local % volume.counts.x, (local / volume.counts.x) % volume.counts.y, local / (volume.counts.x * volume.counts.y));
            return volume.origin.xyz + vec3<f32>(at) * volume.spacing.xyz;
        }
    }
    return vec3(0.0);
}
// The light around the scene along `d`: the environment's, or the
// hemisphere's sky above and ground below.
fn sky_along(d: vec3<f32>) -> vec3<f32> {
    var light = select(frame.ground.rgb, frame.sky.rgb, d.y >= 0.0);
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     let turned = vec3(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
//E     light = textureSampleLevel(environment_cube, environment_sampler, turned, 1.0).rgb * frame.environment.x;
    return light;
}
// The diffuse light over π the world gives a surface facing `n` where no
// probe reaches: the hemisphere's or environment's.
fn around(n: vec3<f32>) -> vec3<f32> {
    var light = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     let t = vec3(c * n.x - s * n.z, n.y, s * n.x + c * n.z);
//E     let k = frame.irradiance;
//E     light = (k[0].rgb + k[1].rgb * t.y + k[2].rgb * t.z + k[3].rgb * t.x
//E         + k[4].rgb * (t.x * t.y) + k[5].rgb * (t.y * t.z) + k[6].rgb * (3.0 * t.z * t.z - 1.0)
//E         + k[7].rgb * (t.x * t.z) + k[8].rgb * (t.x * t.x - t.y * t.y)) * frame.environment.x;
    return light;
}
// Light `i`'s light reaching a surface at `at` facing `n`, unshadowed, and
// how far (a light at the surface itself: none).
fn lamp(i: u32, at: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let point = lights[i];
    let to = point[0].xyz - at;
    let distance = max(length(to), 1e-4);
    let l = to / distance;
    let n_l = dot(n, l);
    if n_l <= 0.0 || distance >= point[0].w {
        return vec4(0.0);
    }
    let falloff = pow(clamp(1.0 - pow(distance / point[0].w, 4.0), 0.0, 1.0), 2.0) / (distance * distance);
    let cone = clamp(dot(point[2].xyz, -l) + point[2].w, 0.0, 1.0);
    return vec4(point[1].rgb * point[1].w * falloff * cone * cone * n_l, distance);
}
// The way from `at` to light `i`, `distance` away (`lamp`'s, never 0).
fn toward(i: u32, at: vec3<f32>, distance: f32) -> vec3<f32> {
    return (lights[i][0].xyz - at) / distance;
}
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
fn hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}
