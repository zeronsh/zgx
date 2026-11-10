
struct Frame {
    view_projection: mat4x4<f32>,
    camera: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    points: array<array<vec4<f32>, 3>, 8>,
    params: vec4<f32>,
    shadow: mat4x4<f32>,
    shadow_texel: vec4<f32>,
    environment: vec4<f32>,
    irradiance: array<vec4<f32>, 9>,
    look: vec4<f32>,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
//E @group(0) @binding(3) var environment_cube: texture_cube<f32>;
//E @group(0) @binding(4) var environment_sampler: sampler;
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
//P @group(2) @binding(0) var<storage, read> palette: array<mat4x4<f32>>;
//M @group(3) @binding(0) var<storage, read> morphs: array<vec4<f32>>;
//M struct Morphed {
//M     position: vec3<f32>,
//M     normal: vec3<f32>,
//M     joints: u32,
//M }
//M // A vertex blended by the mesh's weights (from `base`: how many, how many
//M // targets, then `[target, weight]`s, four a matrix), and where its
//M // joints' matrices start.
//M fn morph(base: u32, vertex: u32, position: vec3<f32>, normal: vec3<f32>) -> Morphed {
//M     let head = palette[base][0];
//M     let blending = u32(head.x);
//M     let targets = u32(head.y);
//M     var out = Morphed(position, normal, base + 1u + (blending + 3u) / 4u);
//M     for (var i = 0u; i < blending; i++) {
//M         let pair = palette[base + 1u + i / 4u][i % 4u];
//M         let at = (vertex * targets + u32(pair.x)) * 2u;
//M         out.position += morphs[at].xyz * pair.y;
//M         out.normal += morphs[at + 1u].xyz * pair.y;
//M     }
//M     return out;
//M }
//S // A vertex's place from its joints' matrices (from `base`), weighted.
//S fn skinned(base: u32, joints: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
//S     return palette[base + joints.x] * weights.x + palette[base + joints.y] * weights.y
//S         + palette[base + joints.z] * weights.z + palette[base + joints.w] * weights.w;
//S }

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) emissive: vec3<f32>,
    @location(4) material: vec3<f32>,
    @location(5) uv: vec2<f32>,
//U     @location(6) second_uv: vec2<f32>,
//G     @location(7) scale: f32,
}

@vertex fn vertex(
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>,
    @location(7) n0: vec4<f32>, @location(8) n1: vec4<f32>, @location(9) n2: vec4<f32>,
    @location(10) color: vec4<f32>, @location(11) emissive: vec4<f32>,
//S     @location(12) joints: vec4<u32>, @location(13) weights: vec4<f32>,
//U     @location(14) second_uv: vec2<f32>,
//C     @location(15) vertex_color: vec4<f32>,
//M     @builtin(vertex_index) index: u32,
) -> Out {
    var model = mat4x4(m0, m1, m2, m3);
    var normals = mat3x3(n0.xyz, n1.xyz, n2.xyz);
    var at = position;
    var facing = normal;
    var joints_from = u32(emissive.w);
//M     let morphed = morph(joints_from, index, position, normal);
//M     at = morphed.position;
//M     facing = morphed.normal;
//M     joints_from = morphed.joints;
//S     model = skinned(joints_from, joints, weights);
//S     normals = mat3x3(model[0].xyz, model[1].xyz, model[2].xyz);
    let world = model * vec4(at, 1.0);
    let n = normals * facing;
    var tint = color;
//C     tint *= vertex_color;
    return Out(
        frame.view_projection * world, world.xyz, n, tint, emissive.xyz, vec3(n0.w, n1.w, n2.w), uv,
//U         second_uv,
//G         length(model[0].xyz),
    );
}

@vertex fn shadow_vertex(
    @location(0) position: vec3<f32>,
    @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>,
//P     @location(11) emissive: vec4<f32>,
//S     @location(12) joints: vec4<u32>, @location(13) weights: vec4<f32>,
//M     @builtin(vertex_index) index: u32,
//U     @location(14) second_uv: vec2<f32>,
//K     @location(2) uv: vec2<f32>, @location(9) n2: vec4<f32>,
//N ) -> @builtin(position) vec4<f32> {
//K ) -> ShadowOut {
    var model = mat4x4(m0, m1, m2, m3);
    var at = position;
//P     var joints_from = u32(emissive.w);
//M     let morphed = morph(joints_from, index, position, vec3(0.0));
//M     at = morphed.position;
//M     joints_from = morphed.joints;
//S     model = skinned(joints_from, joints, weights);
//N     return frame.shadow * model * vec4(at, 1.0);
//K     var cut_uv = uv;
//K //U     cut_uv = placed(0u, uv, second_uv);
//K     return ShadowOut(frame.shadow * model * vec4(at, 1.0), cut_uv, n2.w);
}
//K struct ShadowOut {
//K     @builtin(position) position: vec4<f32>,
//K     @location(0) uv: vec2<f32>,
//K     @location(1) word: f32,
//K }
//K // A surface's alpha cutoff from its third material word (`surface_word`).
//K fn cutoff_of(word: f32) -> f32 {
//K     return select(-word, word - 1.0, word > 0.5);
//K }
//K @fragment fn shadow_fragment(in: ShadowOut) {
//K     if textureSample(base_texture, map_sampler, in.uv).a < cutoff_of(in.word) {
//K         discard;
//K     }
//K }

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
    let h = normalize(v + l);
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
//R     let h = normalize(v + l);
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
fn neutral(color: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desaturation = 0.15;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = color - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3(new_peak), g);
}
//E // Where the environment's light along `d` comes from: the room turned
//E // by its rotation about the vertical.
//E fn turned(d: vec3<f32>) -> vec3<f32> {
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     return vec3(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
//E }
//E // Its diffuse light over π facing `n` (nine spherical harmonics).
//E fn irradiance(n: vec3<f32>) -> vec3<f32> {
//E     let k = frame.irradiance;
//E     return k[0].rgb + k[1].rgb * n.y + k[2].rgb * n.z + k[3].rgb * n.x
//E         + k[4].rgb * (n.x * n.y) + k[5].rgb * (n.y * n.z) + k[6].rgb * (3.0 * n.z * n.z - 1.0)
//E         + k[7].rgb * (n.x * n.z) + k[8].rgb * (n.x * n.x - n.y * n.y);
//E }
//E // The split sum's second factor, the specular's scale and bias by
//E // angle and roughness: Karis's fit to its table (no texture).
//E fn reflectance(f0: vec3<f32>, roughness: f32, n_v: f32) -> vec3<f32> {
//E     let r = roughness * vec4(-1.0, -0.0275, -0.572, 0.022) + vec4(1.0, 0.0425, 1.04, -0.04);
//E     let a004 = min(r.x * r.x, exp2(-9.28 * n_v)) * r.x + r.y;
//E     let ab = vec2(-1.04, 1.04) * a004 + r.zw;
//E     return f0 * ab.x + ab.y;
//E }
// ACES's filmic curve, Stephen Hill's fit: into its working space, its
// reference transform and display transform as one rational curve, back.
// Exposed by 1 / 0.6 first, as three.js does, so mid grey stays about mid.
fn aces(color: vec3<f32>) -> vec3<f32> {
    let into = mat3x3(vec3(0.59719, 0.07600, 0.02840), vec3(0.35458, 0.90834, 0.13383), vec3(0.04823, 0.01566, 0.83777));
    let back = mat3x3(vec3(1.60475, -0.10208, -0.00327), vec3(-0.53108, 1.10813, -0.07276), vec3(-0.07367, -0.00605, 1.07602));
    let v = into * (color / 0.6);
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(back * (a / b), vec3(0.0), vec3(1.0));
}
fn tone_map(color: vec3<f32>) -> vec3<f32> {
    if frame.look.x > 1.5 {
        return color;
    }
    if frame.look.x > 0.5 {
        return aces(color);
    }
    return neutral(color);
}
fn encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}
//G fn decode(c: vec3<f32>) -> vec3<f32> {
//G     let low = c / 12.92;
//G     let high = pow((c + 0.055) / 1.055, vec3(2.4));
//G     return select(high, low, c <= vec3(0.04045));
//G }

//T // The surface's normal tilted by a normal map's texel (0 to 1 each, half
//T // way flat), its frame from the screen's derivatives of the point and its
//T // texture coordinates (as Khronos's viewer makes it without tangents).
//T fn bumped(ng: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, duvx: vec2<f32>, duvy: vec2<f32>, texel: vec3<f32>) -> vec3<f32> {
//T     let det = duvx.x * duvy.y - duvy.x * duvx.y;
//T     let along = (duvy.y * dpx - duvx.y * dpy) * sign(det);
//T     let t = along - ng * dot(ng, along);
//T     if abs(det) < 1e-12 || dot(t, t) < 1e-20 {
//T         return ng;
//T     }
//T     let tangent = normalize(t);
//T     let m = texel * 2.0 - 1.0;
//T     return normalize(tangent * m.x + cross(ng, tangent) * m.y + ng * m.z);
//T }

@fragment fn fragment(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Its maps (none in an untextured pipeline): its colour (white without
    // one), in linear light; metal and roughness; which way it faces.
    var texel = vec4(1.0);
    var surface = vec4(1.0);
    var bump = vec3(0.5, 0.5, 1.0);
    var glow = vec3(1.0);
    var occlusion = 1.0;
//T     var uvs = array<vec2<f32>, 5>(in.uv, in.uv, in.uv, in.uv, in.uv);
//U     for (var i = 0u; i < 5u; i++) {
//U         uvs[i] = placed(i, in.uv, in.second_uv);
//U     }
//T     texel = textureSample(base_texture, map_sampler, uvs[0]);
//T     surface = textureSample(metal_rough_texture, map_sampler, uvs[1]);
//T     bump = textureSample(normal_texture, map_sampler, uvs[2]).xyz;
//T     glow = textureSample(emission_texture, map_sampler, uvs[3]).rgb;
//T     occlusion = textureSample(occlusion_texture, map_sampler, uvs[4]).r;
//G     let passing = textureSample(transmission_texture, map_sampler, in.uv).r;
//G     let thick = textureSample(thickness_texture, map_sampler, in.uv).g;
//L     let coat_amount = layers.coat.x * textureSample(coat_texture, map_sampler, in.uv).r;
//L     let coat_given = clamp(layers.coat.y * textureSample(coat_roughness_texture, map_sampler, in.uv).g, 0.02, 1.0);
//L     let coat_bump = textureSample(coat_normal_texture, map_sampler, in.uv).xyz;
//L     let coat_duvx = dpdx(in.uv);
//L     let coat_duvy = dpdy(in.uv);
//T     let dpx = dpdx(in.world);
//T     let dpy = dpdy(in.world);
//T     let duvx = dpdx(uvs[2]);
//T     let duvy = dpdy(uvs[2]);
    // How fast the surface's normal turns across the pixel, in uniform
    // control flow: widened into its roughness below (geometric specular
    // anti-aliasing, Tokuyoshi and Kaplanyan), so a highlight on a curve
    // too fine for the pixels blurs instead of flickering.
    // Multisampled pipelines' alone: one drawing a sample a pixel skips it.
    var widening = 0.0;
//A let facing = normalize(in.normal);
//A let turning_x = dpdx(facing);
//A let turning_y = dpdy(facing);
//A widening = min(0.3 * (dot(turning_x, turning_x) + dot(turning_y, turning_y)), 0.18);
//K     if texel.a < cutoff_of(in.material.z) {
//K         discard;
//K     }
    var base = in.color.rgb * texel.rgb;
    var lit: vec3<f32>;
    if in.material.z > 0.5 {
        lit = base;
    } else {
        var n = normalize(in.normal);
        if !front {
            n = -n;
        }
        // A coat faces as its own normal map says, not as the surface's
        // under it does.
//L         let coat_n = bumped(n, dpx, dpy, coat_duvx, coat_duvy, coat_bump);
//T         n = bumped(n, dpx, dpy, duvx, duvy, bump);
        let v = normalize(frame.camera.xyz - in.world);
        let metallic = in.material.x * surface.b;
        let given = clamp(in.material.y * surface.g, 0.02, 1.0);
        let a = given * given;
        let roughness = sqrt(sqrt(clamp(a * a + widening, 0.0, 1.0)));
//L         let coat_a = coat_given * coat_given;
//L         let coat_rough = sqrt(sqrt(clamp(coat_a * coat_a + widening, 0.0, 1.0)));
        // What passes through takes the place of its diffuse light.
//G         let see_through = layers.a.x * passing * (1.0 - metallic);
//G         let tinted = base;
//G         base *= 1.0 - see_through;
        var sun = frame.sun_color.rgb;
        if frame.params.w > 0.5 {
            sun *= sunlit(in.world, n);
        }
//Q         lit = shade(base, metallic, roughness, n, v, -frame.sun_direction.xyz, sun);
//R         lit = shade_specular(base, metallic, roughness, n, v, -frame.sun_direction.xyz, sun);
        // A coat's own highlights: glass's (a black dielectric's alone).
//L         var coat_lit = shade(vec3(0.0), 0.0, coat_rough, coat_n, v, -frame.sun_direction.xyz, sun);
        for (var i = 0u; i < u32(frame.params.x); i++) {
            let light = frame.points[i];
            let to = light[0].xyz - in.world;
            let distance = length(to);
            let falloff = pow(clamp(1.0 - pow(distance / light[0].w, 4.0), 0.0, 1.0), 2.0) / max(distance * distance, 1e-4);
            // Within a spot's cone (all round: 1).
            let cone = clamp(dot(light[2].xyz, -to / distance) + light[2].w, 0.0, 1.0);
//Q             lit += shade(base, metallic, roughness, n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
//R             lit += shade_specular(base, metallic, roughness, n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
//L             coat_lit += shade(vec3(0.0), 0.0, coat_rough, coat_n, v, to / distance, light[1].rgb * light[1].w * falloff * cone * cone);
        }
        // Ambient light: the hemisphere seen by the surface for its diffuse
        // part, and along its reflection, by Fresnel, for its specular one
        // (what makes metals their colour away from the lights).
        let ambient = mix(frame.ground.rgb, frame.sky.rgb, n.y * 0.5 + 0.5);
        let r = reflect(-v, n);
        let reflected = mix(frame.ground.rgb, frame.sky.rgb, r.y * 0.5 + 0.5);
//Q         let f0 = mix(vec3(0.04), base, metallic);
//R         let f0 = mix(layers.spec.w * min(layers.spec.rgb, vec3(1.0)), base, metallic);
        let n_v = max(dot(n, v), 0.0);
        let rough = roughness * roughness;
//Q         let fresnel = f0 + (max(vec3(1.0 - rough), f0) - f0) * pow(1.0 - n_v, 5.0);
//R         let fresnel = f0 + (max(vec3(1.0 - rough) * mix(layers.spec.w, 1.0, metallic), f0) - f0) * pow(1.0 - n_v, 5.0);
        var surround = ambient * base * (1.0 - metallic) * (1.0 - fresnel);
        surround += reflected * fresnel * (1.0 - 0.5 * rough);
        // Lit by an environment instead: its diffuse light, and its light
        // along the reflection blurred as rough as the surface (the cube's
        // level) times the split sum's factor.
//E     let along = textureSampleLevel(environment_cube, environment_sampler, turned(r), roughness * frame.environment.w).rgb;
//E     surround = irradiance(turned(n)) * base * (1.0 - metallic) * (1.0 - fresnel);
//E     surround = (surround + along * reflectance(f0, roughness, n_v)) * frame.environment.x;
        lit += surround * occlusion;
        lit += in.emissive * glow;
        // A coat reflects its surroundings as glass, and what is under it
        // (its light included) shows the less the more it reflects.
//L         let coat_r = reflect(-v, coat_n);
//L         let coat_n_v = max(dot(coat_n, v), 0.0);
//L         let coat_rough2 = coat_rough * coat_rough;
//L         let coat_fresnel = 0.04 + (max(1.0 - coat_rough2, 0.04) - 0.04) * pow(1.0 - coat_n_v, 5.0);
//L         var coat_around = mix(frame.ground.rgb, frame.sky.rgb, coat_r.y * 0.5 + 0.5) * coat_fresnel * (1.0 - 0.5 * coat_rough2);
//E //L     coat_around = textureSampleLevel(environment_cube, environment_sampler, turned(coat_r), coat_rough * frame.environment.w).rgb * reflectance(vec3(0.04), coat_rough, coat_n_v) * frame.environment.x;
//W         var under = 1.0;
//L         under = 1.0 - coat_amount * (0.04 + 0.96 * pow(1.0 - coat_n_v, 5.0));
//L         lit = lit * under + coat_amount * (coat_lit + coat_around * occlusion);
        lit = tone_map(lit * frame.params.z);
        // What shows through, behind already mapped, less what reflects.
//G         let facing_f0 = pow((layers.a.y - 1.0) / (layers.a.y + 1.0), 2.0);
//G         let reflects = facing_f0 + (1.0 - facing_f0) * pow(1.0 - n_v, 5.0);
//G         lit += through(in.world, n, v, roughness, in.scale * thick) * tinted * see_through * (1.0 - reflects) * under;
    }
    if frame.params.y > 0.5 {
        lit = encode(clamp(lit, vec3(0.0), vec3(1.0)));
    }
    // A translucent mesh's texture adds its own alpha; an opaque one's
    // is ignored (glTF's OPAQUE).
    let alpha = clamp(select(1.0, in.color.a * texel.a, in.color.a < 1.0), 0.0, 1.0);
    return vec4(lit * alpha, alpha);
}
