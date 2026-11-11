// Irradiance probes (`gi` feature, `World::add_probes`): a world's
// volumes' probes, each an octahedral map of the light arriving at it (6
// by 6 texels inside a border of 1, as diffuse light over π) and of how far
// the surfaces around it are (14 by 14, distance and its square), in two
// atlases of 64 probes a row. Read by every lit mesh of a world with
// volumes; the probes' tracer (`probes_trace.wgsl`) reads them too, these
// lines with their tag taken off.
//probed struct ProbeVolume {
//probed     // The first probe's place; its first probe's index in the atlases.
//probed     origin: vec4<f32>,
//probed     // From one probe to the next (0 on an axis of one probe).
//probed     spacing: vec4<f32>,
//probed     // Its inverse (0 on an axis of one probe); how far off a surface a
//probed     // lookup starts, toward the eye and along the normal.
//probed     inverse: vec4<f32>,
//probed     counts: vec4<u32>,
//probed     // Where it lights fully, and how far past that it fades out.
//probed     lo: vec4<f32>,
//probed     hi: vec4<f32>,
//probed     fade: vec4<f32>,
//probed }
//probed struct ProbeSet {
//probed     volumes: array<ProbeVolume, 4>,
//probed     // How many volumes; the atlases' texel sizes (irradiance, depth).
//probed     count: vec4<u32>,
//probed     texel: vec4<f32>,
//probed }
//probed @group(0) @binding(9) var probe_irradiance: texture_2d<f32>;
//probed @group(0) @binding(10) var probe_depth: texture_2d<f32>;
//probed @group(0) @binding(11) var<uniform> probe_set: ProbeSet;
//probed @group(0) @binding(12) var probe_sampler: sampler;
//probed // `d` (a unit vector) on the octahedron unfolded to a square, -1 to 1.
//probed fn octahedral(d: vec3<f32>) -> vec2<f32> {
//probed     let p = d.xy / (abs(d.x) + abs(d.y) + abs(d.z));
//probed     if d.z >= 0.0 {
//probed         return p;
//probed     }
//probed     return (1.0 - abs(p.yx)) * select(vec2(-1.0), vec2(1.0), p >= vec2(0.0));
//probed }
//probed // Where probe `index`'s map of `tile` texels a side (`tile - 2` inside
//probed // its border) holds direction `d`, in the atlas's coordinates.
//probed fn probe_uv(index: u32, d: vec3<f32>, tile: f32, texel: vec2<f32>) -> vec2<f32> {
//probed     let corner = vec2(f32(index % 64u), f32(index / 64u)) * tile;
//probed     return (corner + 1.0 + (octahedral(d) * 0.5 + 0.5) * (tile - 2.0)) * texel;
//probed }
//probed // The diffuse light over π at `p` facing `n`, seen along `v` (toward
//probed // the eye) from the probes about it, and how fully the volume lights it
//probed // (0 outside every volume). Each of the eight probes around it weighs by
//probed // how near it is, whether it's in front of the surface, and whether it
//probed // can see `p` (its distances' mean and variance, Chebyshev's bound): so
//probed // light from the far side of a wall doesn't come through it.
//probed fn probe_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec4<f32> {
//probed     for (var i = 0u; i < probe_set.count.x; i++) {
//probed         let volume = probe_set.volumes[i];
//probed         let outside = max(max(volume.lo.xyz - p, p - volume.hi.xyz), vec3(0.0)) / volume.fade.xyz;
//probed         let fade = saturate(1.0 - max(outside.x, max(outside.y, outside.z)));
//probed         if fade <= 0.0 {
//probed             continue;
//probed         }
//probed         let off = p + (n * 0.2 + v * 0.8) * volume.inverse.w;
//probed         let grid = (off - volume.origin.xyz) * volume.inverse.xyz;
//probed         let last = vec3<f32>(volume.counts.xyz - 1u);
//probed         let base = clamp(floor(grid), vec3(0.0), max(last - 1.0, vec3(0.0)));
//probed         let t = clamp(grid - base, vec3(0.0), vec3(1.0));
//probed         var sum = vec3(0.0);
//probed         var weights = 0.0;
//probed         for (var corner = 0u; corner < 8u; corner++) {
//probed             let step = vec3<f32>(vec3(corner & 1u, (corner >> 1u) & 1u, (corner >> 2u) & 1u));
//probed             let at = min(base + step, last);
//probed             let index = u32(volume.origin.w) + u32(at.x) + volume.counts.x * (u32(at.y) + volume.counts.y * u32(at.z));
//probed             let probe = volume.origin.xyz + at * volume.spacing.xyz;
//probed             let trilinear = mix(1.0 - t, t, step);
//probed             // In front of the surface: more; behind it, a little.
//probed             let toward = normalize(probe - p + vec3(1e-6));
//probed             let facing = (dot(toward, n) + 1.0) * 0.5;
//probed             var weight = facing * facing + 0.2;
//probed             // Seen from the probe: the surfaces' distances that way.
//probed             let out = off - probe;
//probed             let distance = length(out);
//probed             let moments = textureSampleLevel(probe_depth, probe_sampler, probe_uv(index, out / max(distance, 1e-6), 16.0, probe_set.texel.zw), 0.0).xy;
//probed             if distance > moments.x {
//probed                 let variance = abs(moments.y - moments.x * moments.x);
//probed                 let past = distance - moments.x;
//probed                 let chebyshev = variance / (variance + past * past);
//probed                 weight *= max(chebyshev * chebyshev * chebyshev, 0.05);
//probed             }
//probed             weight = max(weight, 1e-6);
//probed             // Probes all but unseen count for next to nothing.
//probed             if weight < 0.2 {
//probed                 weight *= weight * weight * 25.0;
//probed             }
//probed             weight *= trilinear.x * trilinear.y * trilinear.z;
//probed             let light = textureSampleLevel(probe_irradiance, probe_sampler, probe_uv(index, n, 8.0, probe_set.texel.xy), 0.0).rgb;
//probed             sum += light * weight;
//probed             weights += weight;
//probed         }
//probed         return vec4(sum / max(weights, 1e-9), fade);
//probed     }
//probed     return vec4(0.0);
//probed }
