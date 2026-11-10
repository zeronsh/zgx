// The sun's cascades (`render/shadows/cascades.rs`): up to four maps, a
// layer each of one texture, each with its view onto it.

//H struct Cascades {
//H     matrices: array<mat4x4<f32>, 4>,
//H     // A lookup's offset off a surface facing the sun, and how much less
//H     // the more it faces it (as `shadow_texel`'s z and w).
//H     offsets: array<vec4<f32>, 4>,
//H     // How many, and a texel's width in a map (1 / side).
//H     params: vec4<f32>,
//H }
//H @group(0) @binding(6) var<uniform> cascades: Cascades;
//H @group(0) @binding(7) var cascade_maps: texture_depth_2d_array;
//H // How much of the sun reaches `world` (facing `n`), from the first
//H // cascade holding it with room for the filter: four filtered
//H // comparisons half a texel about it, as `sunlit`'s. Past the last, lit.
//H fn sunlit_cascaded(world: vec3<f32>, n: vec3<f32>) -> f32 {
//H     let facing = saturate(dot(n, -frame.sun_direction.xyz));
//H     let texel = cascades.params.y;
//H     for (var c = 0u; c < u32(cascades.params.x); c++) {
//H         let offset = cascades.offsets[c];
//H         let p = cascades.matrices[c] * vec4(world + n * (offset.z - offset.w * facing), 1.0);
//H         let uv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
//H         if all(uv > vec2(texel * 2.0)) && all(uv < vec2(1.0 - texel * 2.0)) && p.z <= 1.0 {
//H             let half = texel * 0.5;
//H             var sum = 0.0;
//H             sum += textureSampleCompareLevel(cascade_maps, shadow_sampler, uv + vec2(-half, -half), c, p.z);
//H             sum += textureSampleCompareLevel(cascade_maps, shadow_sampler, uv + vec2(half, -half), c, p.z);
//H             sum += textureSampleCompareLevel(cascade_maps, shadow_sampler, uv + vec2(-half, half), c, p.z);
//H             sum += textureSampleCompareLevel(cascade_maps, shadow_sampler, uv + vec2(half, half), c, p.z);
//H             return sum * 0.25;
//H         }
//H     }
//H     return 1.0;
//H }
