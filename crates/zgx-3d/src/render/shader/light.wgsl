// How much light reaches a surface (the sun's shadow), and how a
// surface reflects it.

const PI: f32 = 3.14159265;

// How much of the sun reaches `world` (facing `n`): 1 lit, 0 in shadow,
// between at a shadow's edge (four filtered comparisons half a texel
// about it: each blends 2 by 2 texels, so 3 by 3 in all).
fn sunlit(world: vec3<f32>, n: vec3<f32>) -> f32 {
    // Off the surface by a texel or so, against its own shadow; more as it
    // turns from the sun, where a coarse curve's flat faces fall behind
    // its smooth normals (the frame's numbers: no literals to parse).
    let facing = saturate(dot(n, -frame.sun_direction.xyz));
    let p = frame.shadow * vec4(world + n * (frame.shadow_texel.z - frame.shadow_texel.w * facing), 1.0);
    let uv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || p.z > 1.0 {
        return 1.0;
    }
    let half = frame.shadow_texel.y * 0.5;
    var sum = 0.0;
    sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2(-half, -half), p.z);
    sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2(half, -half), p.z);
    sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2(-half, half), p.z);
    sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2(half, half), p.z);
    return sum * 0.25;
}

fn ggx(n_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_h * n_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}
fn smith(n_v: f32, n_l: f32, a: f32) -> f32 {
    let k = a * 0.5;
    return (n_v / (n_v * (1.0 - k) + k)) * (n_l / (n_l * (1.0 - k) + k));
}
fn shade(base: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>) -> vec3<f32> {
    let h = (v + l) / max(length(v + l), 1e-4);
    let n_l = max(dot(n, l), 0.0);
    let n_v = max(dot(n, v), 1e-4);
    let n_h = max(dot(n, h), 0.0);
    let a = roughness * roughness;
    let f0 = mix(vec3(0.04), base, metallic);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    let specular = fresnel * ggx(n_h, a) * smith(n_v, n_l, a) / (4.0 * n_v * max(n_l, 1e-4));
    let diffuse = (1.0 - fresnel) * (1.0 - metallic) * base / PI;
    return (diffuse + specular) * radiance * n_l;
}
//R // `shade` for a dielectric reflecting as its specular says
//R // (`layers.spec`): as much facing it as its index of refraction and
//R // colour say, and that and its grazing reflection scaled by its factor.
//R fn shade_specular(base: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>) -> vec3<f32> {
//R     let h = (v + l) / max(length(v + l), 1e-4);
//R     let n_l = max(dot(n, l), 0.0);
//R     let n_v = max(dot(n, v), 1e-4);
//R     let n_h = max(dot(n, h), 0.0);
//R     let a = roughness * roughness;
//R     let f0 = mix(layers.spec.w * min(layers.spec.rgb, vec3(1.0)), base, metallic);
//R     let f90 = mix(layers.spec.w, 1.0, metallic);
//R     let fresnel = f0 + (f90 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
//R     let specular = fresnel * ggx(n_h, a) * smith(n_v, n_l, a) / (4.0 * n_v * max(n_l, 1e-4));
//R     let diffuse = (1.0 - fresnel) * (1.0 - metallic) * base / PI;
//R     return (diffuse + specular) * radiance * n_l;
//R }
