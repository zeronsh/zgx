// A material's maps and layers (group 1), and what shows through glass.

//T @group(1) @binding(0) var base_texture: texture_2d<f32>;
//T @group(1) @binding(1) var metal_rough_texture: texture_2d<f32>;
//T @group(1) @binding(2) var normal_texture: texture_2d<f32>;
//T @group(1) @binding(3) var emission_texture: texture_2d<f32>;
//T @group(1) @binding(4) var occlusion_texture: texture_2d<f32>;
//T @group(1) @binding(5) var map_sampler: sampler;
//U struct Placement {
//U     u: vec4<f32>,
//U     v: vec4<f32>,
//U }
//U @group(1) @binding(6) var<uniform> placements: array<Placement, 5>;
//U // Where map `i` reads: its coordinates (the second, if its row says)
//U // through its rows.
//U fn placed(i: u32, first: vec2<f32>, second: vec2<f32>) -> vec2<f32> {
//U     let p = placements[i];
//U     let uv = vec3(select(first, second, p.u.w > 0.5), 1.0);
//U     return vec2(dot(p.u.xyz, uv), dot(p.v.xyz, uv));
//U }
//W struct Layers {
//W     // How much passes through, its index of refraction, thickness and
//W     // attenuation distance; its attenuation colour; how much coat and
//W     // how rough; how much a dielectric reflects facing it, by channel,
//W     // and how much of that.
//W     a: vec4<f32>,
//W     b: vec4<f32>,
//W     coat: vec4<f32>,
//W     spec: vec4<f32>,
//W }
//W @group(1) @binding(7) var<uniform> layers: Layers;
//G @group(1) @binding(8) var behind: texture_2d<f32>;
//G @group(1) @binding(9) var behind_sampler: sampler;
//G @group(1) @binding(10) var transmission_texture: texture_2d<f32>;
//G @group(1) @binding(11) var thickness_texture: texture_2d<f32>;
//L @group(1) @binding(12) var coat_texture: texture_2d<f32>;
//L @group(1) @binding(13) var coat_roughness_texture: texture_2d<f32>;
//L @group(1) @binding(14) var coat_normal_texture: texture_2d<f32>;
//G // What shows through at `world`, seen along `v`: what is behind, bent by
//G // the index of refraction through the thickness (`scale` its mesh's),
//G // blurred as rough as the surface, the light lost on the way taken out.
//G fn through(world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, roughness: f32, scale: f32) -> vec3<f32> {
//G     let ior = layers.a.y;
//G     let ray = normalize(refract(-v, n, 1.0 / ior)) * layers.a.z * scale;
//G     let seen = frame.view_projection * vec4(world + ray, 1.0);
//G     let uv = vec2(0.5, -0.5) * seen.xy / seen.w + 0.5;
//G     let level = f32(textureNumLevels(behind) - 1u) * roughness * clamp(ior * 2.0 - 2.0, 0.0, 1.0);
//G     var color = textureSampleLevel(behind, behind_sampler, uv, level).rgb;
//G     if frame.params.y > 0.5 {
//G         color = decode(color);
//G     }
//G     // Beer's law: falling to the attenuation colour every distance.
//G     let lost = -log(layers.b.rgb) / layers.a.w;
//G     return color * exp(-lost * length(ray));
//G }
