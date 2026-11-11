// Irradiance probes (`gi` feature, `World::add_probes`): a world's
// volumes' probes, each an octahedral map of the light arriving at it (6
// by 6 texels inside a border of 1, as diffuse light over π) and of how far
// the surfaces around it are (14 by 14, distance and its square), in two
// atlases of 64 probes a row. Read by every lit mesh of a world with
// volumes; the probes' tracer (`probes_trace.wgsl`) reads them too, these
// lines with their tag taken off.
//g struct ProbeVolume {
//g     // The first probe's place; its first probe's index in the atlases.
//g     origin: vec4<f32>,
//g     // From one probe to the next (0 on an axis of one probe).
//g     spacing: vec4<f32>,
//g     // Its inverse (0 on an axis of one probe); how far off a surface a
//g     // lookup starts, toward the eye and along the normal.
//g     inverse: vec4<f32>,
//g     counts: vec4<u32>,
//g     // Where it lights fully, and how far past that it fades out.
//g     lo: vec4<f32>,
//g     hi: vec4<f32>,
//g     fade: vec4<f32>,
//g }
//g struct ProbeSet {
//g     volumes: array<ProbeVolume, 4>,
//g     // How many volumes; the atlases' texel sizes (irradiance, depth).
//g     count: vec4<u32>,
//g     texel: vec4<f32>,
//g }
//g @group(0) @binding(9) var probe_irradiance: texture_2d<f32>;
//g @group(0) @binding(10) var probe_depth: texture_2d<f32>;
//g @group(0) @binding(11) var<uniform> probe_set: ProbeSet;
//g @group(0) @binding(12) var probe_sampler: sampler;
//g // `d` (a unit vector) on the octahedron unfolded to a square, -1 to 1.
//g fn octahedral(d: vec3<f32>) -> vec2<f32> {
//g     let p = d.xy / (abs(d.x) + abs(d.y) + abs(d.z));
//g     if d.z >= 0.0 {
//g         return p;
//g     }
//g     return (1.0 - abs(p.yx)) * select(vec2(-1.0), vec2(1.0), p >= vec2(0.0));
//g }
//g // Where probe `index`'s map of `tile` texels a side (`tile - 2` inside
//g // its border) holds direction `d`, in the atlas's coordinates.
//g fn probe_uv(index: u32, d: vec3<f32>, tile: f32, texel: vec2<f32>) -> vec2<f32> {
//g     let corner = vec2(f32(index % 64u), f32(index / 64u)) * tile;
//g     return (corner + 1.0 + (octahedral(d) * 0.5 + 0.5) * (tile - 2.0)) * texel;
//g }
//g // The diffuse light over π at `p` facing `n`, seen along `v` (toward
//g // the eye) from the probes about it, and how fully the volume lights it
//g // (0 outside every volume). Each of the eight probes around it weighs by
//g // how near it is, whether it's in front of the surface, and whether it
//g // can see `p` (its distances' mean and variance, Chebyshev's bound): so
//g // light from the far side of a wall doesn't come through it.
//g fn probe_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec4<f32> {
//g     for (var i = 0u; i < probe_set.count.x; i++) {
//g         let volume = probe_set.volumes[i];
//g         let outside = max(max(volume.lo.xyz - p, p - volume.hi.xyz), vec3(0.0)) / volume.fade.xyz;
//g         let fade = saturate(1.0 - max(outside.x, max(outside.y, outside.z)));
//g         if fade <= 0.0 {
//g             continue;
//g         }
//g         let off = p + (n * 0.2 + v * 0.8) * volume.inverse.w;
//g         let grid = (off - volume.origin.xyz) * volume.inverse.xyz;
//g         let last = vec3<f32>(volume.counts.xyz - 1u);
//g         let base = clamp(floor(grid), vec3(0.0), max(last - 1.0, vec3(0.0)));
//g         let t = clamp(grid - base, vec3(0.0), vec3(1.0));
//g         var sum = vec3(0.0);
//g         var weights = 0.0;
//g         for (var corner = 0u; corner < 8u; corner++) {
//g             let step = vec3<f32>(vec3(corner & 1u, (corner >> 1u) & 1u, (corner >> 2u) & 1u));
//g             let at = min(base + step, last);
//g             let index = u32(volume.origin.w) + u32(at.x) + volume.counts.x * (u32(at.y) + volume.counts.y * u32(at.z));
//g             let probe = volume.origin.xyz + at * volume.spacing.xyz;
//g             let trilinear = mix(1.0 - t, t, step);
//g             // In front of the surface: more; behind it, a little.
//g             let toward = normalize(probe - p + vec3(1e-6));
//g             let facing = (dot(toward, n) + 1.0) * 0.5;
//g             var weight = facing * facing + 0.2;
//g             // Seen from the probe: the surfaces' distances that way.
//g             let out = off - probe;
//g             let distance = length(out);
//g             let moments = textureSampleLevel(probe_depth, probe_sampler, probe_uv(index, out / max(distance, 1e-6), 16.0, probe_set.texel.zw), 0.0).xy;
//g             if distance > moments.x {
//g                 let variance = abs(moments.y - moments.x * moments.x);
//g                 let past = distance - moments.x;
//g                 let chebyshev = variance / (variance + past * past);
//g                 weight *= max(chebyshev * chebyshev * chebyshev, 0.05);
//g             }
//g             weight = max(weight, 1e-6);
//g             // Probes all but unseen count for next to nothing.
//g             if weight < 0.2 {
//g                 weight *= weight * weight * 25.0;
//g             }
//g             weight *= trilinear.x * trilinear.y * trilinear.z;
//g             let light = textureSampleLevel(probe_irradiance, probe_sampler, probe_uv(index, n, 8.0, probe_set.texel.xy), 0.0).rgb;
//g             sum += light * weight;
//g             weights += weight;
//g         }
//g         return vec4(sum / max(weights, 1e-9), fade);
//g     }
//g     return vec4(0.0);
//g }
