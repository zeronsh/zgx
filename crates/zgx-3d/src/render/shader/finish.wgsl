// A finish over a surface (`Variant::FINISH`, `crate::finish`): sheen,
// anisotropy and iridescence, each as Khronos's glTF extensions say.

// How much a sheen reflects of light from all about (its directional
// albedo, E): 16 roughnesses (0 to 1) of 16 cosines (0 to 1) each, baked
// from its lobe below by numeric integration (`a_sheens_albedo_is_its_lobes`),
// at most 1.
//finish const ZGX_SHEEN_E = array<f32, 256>(
//finish     1.0000, 0.0590, 0.0002, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
//finish     1.0000, 0.6778, 0.2011, 0.0475, 0.0095, 0.0015, 0.0002, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
//finish     1.0000, 0.7468, 0.4001, 0.2061, 0.1038, 0.0502, 0.0229, 0.0097, 0.0038, 0.0013, 0.0004, 0.0001, 0.0000, 0.0000, 0.0000, 0.0000,
//finish     0.9536, 0.6852, 0.4444, 0.2856, 0.1836, 0.1166, 0.0724, 0.0436, 0.0252, 0.0139, 0.0071, 0.0033, 0.0014, 0.0004, 0.0001, 0.0000,
//finish     0.8227, 0.6365, 0.4557, 0.3260, 0.2347, 0.1684, 0.1195, 0.0833, 0.0568, 0.0378, 0.0242, 0.0147, 0.0082, 0.0040, 0.0016, 0.0003,
//finish     0.7496, 0.6091, 0.4640, 0.3539, 0.2721, 0.2093, 0.1600, 0.1211, 0.0903, 0.0663, 0.0476, 0.0330, 0.0217, 0.0131, 0.0068, 0.0024,
//finish     0.7115, 0.5989, 0.4763, 0.3794, 0.3043, 0.2447, 0.1963, 0.1562, 0.1232, 0.0963, 0.0742, 0.0558, 0.0404, 0.0276, 0.0171, 0.0086,
//finish     0.6961, 0.6016, 0.4939, 0.4060, 0.3354, 0.2781, 0.2303, 0.1898, 0.1553, 0.1265, 0.1021, 0.0810, 0.0625, 0.0463, 0.0321, 0.0197,
//finish     0.6961, 0.6137, 0.5161, 0.4345, 0.3667, 0.3107, 0.2632, 0.2222, 0.1867, 0.1566, 0.1305, 0.1074, 0.0867, 0.0679, 0.0508, 0.0353,
//finish     0.7061, 0.6319, 0.5414, 0.4642, 0.3983, 0.3428, 0.2952, 0.2537, 0.2172, 0.1860, 0.1587, 0.1342, 0.1118, 0.0911, 0.0719, 0.0540,
//finish     0.7222, 0.6534, 0.5678, 0.4940, 0.4295, 0.3739, 0.3259, 0.2837, 0.2463, 0.2142, 0.1859, 0.1603, 0.1367, 0.1147, 0.0941, 0.0746,
//finish     0.7405, 0.6751, 0.5931, 0.5219, 0.4587, 0.4029, 0.3543, 0.3114, 0.2732, 0.2404, 0.2113, 0.1849, 0.1605, 0.1376, 0.1161, 0.0957,
//finish     0.7573, 0.6942, 0.6150, 0.5459, 0.4839, 0.4283, 0.3792, 0.3357, 0.2970, 0.2637, 0.2341, 0.2072, 0.1823, 0.1589, 0.1368, 0.1160,
//finish     0.7691, 0.7076, 0.6309, 0.5639, 0.5033, 0.4483, 0.3991, 0.3555, 0.3166, 0.2830, 0.2533, 0.2263, 0.2012, 0.1777, 0.1555, 0.1346,
//finish     0.7723, 0.7126, 0.6384, 0.5736, 0.5147, 0.4610, 0.4125, 0.3693, 0.3307, 0.2975, 0.2681, 0.2414, 0.2166, 0.1933, 0.1714, 0.1508,
//finish     0.7634, 0.7061, 0.6349, 0.5728, 0.5163, 0.4645, 0.4176, 0.3756, 0.3382, 0.3060, 0.2775, 0.2516, 0.2276, 0.2051, 0.1838, 0.1638,
//finish );
// E at cosine `c` and roughness `r`, bilinearly.
//finish fn zgx_sheen_albedo(c: f32, r: f32) -> f32 {
//finish     let x = clamp(c, 0.0, 1.0) * 15.0;
//finish     let y = clamp(r, 0.0, 1.0) * 15.0;
//finish     let x0 = min(u32(x), 14u);
//finish     let y0 = min(u32(y), 14u);
//finish     let fx = x - f32(x0);
//finish     let fy = y - f32(y0);
//finish     let i = y0 * 16u + x0;
//finish     let top = mix(ZGX_SHEEN_E[i], ZGX_SHEEN_E[i + 1u], fx);
//finish     let bottom = mix(ZGX_SHEEN_E[i + 16u], ZGX_SHEEN_E[i + 17u], fx);
//finish     return mix(top, bottom, fy);
//finish }
// The sheen's lobe: the "Charlie" distribution (Estevez and Kulla 2017)
// and its visibility by their fitted λ, as Khronos's viewer has them.
//finish fn zgx_charlie(n_h: f32, r: f32) -> f32 {
//finish     let a = max(r, 1e-6) * max(r, 1e-6);
//finish     let inv = 1.0 / a;
//finish     let sin2 = max(1.0 - n_h * n_h, 0.0);
//finish     return (2.0 + inv) * pow(sin2, inv * 0.5) / (2.0 * PI);
//finish }
//finish fn zgx_sheen_fit(x: f32, a: f32) -> f32 {
//finish     let t = (1.0 - a) * (1.0 - a);
//finish     let p = mix(21.5473, 25.3245, t);
//finish     let q = mix(3.82987, 3.32435, t);
//finish     let s = mix(0.19823, 0.16801, t);
//finish     let d = mix(-1.97760, -1.27393, t);
//finish     let e = mix(-4.32054, -4.85967, t);
//finish     return p / (1.0 + q * pow(x, s)) + d * x + e;
//finish }
//finish fn zgx_sheen_lambda(c: f32, a: f32) -> f32 {
//finish     if abs(c) < 0.5 {
//finish         return exp(zgx_sheen_fit(c, a));
//finish     }
//finish     return exp(2.0 * zgx_sheen_fit(0.5, a) - zgx_sheen_fit(1.0 - c, a));
//finish }
//finish fn zgx_sheen_visibility(n_l: f32, n_v: f32, r: f32) -> f32 {
//finish     let a = max(r, 1e-6) * max(r, 1e-6);
//finish     let v = 1.0 / ((1.0 + zgx_sheen_lambda(n_v, a) + zgx_sheen_lambda(n_l, a)) * (4.0 * n_v * n_l));
//finish     return clamp(v, 0.0, 1.0);
//finish }

// A thin film's reflectance (Belcour and Barla 2017, as Khronos's viewer
// has it): its index `film_ior` over a surface reflecting `f0` facing it,
// `thickness` nanometres thick, seen at cosine `c` from air.
//finish fn zgx_film_sensitivity(opd: f32, shift: vec3<f32>) -> vec3<f32> {
//finish     let phase = 2.0 * PI * opd * 1.0e-9;
//finish     let val = vec3(5.4856e-13, 4.4201e-13, 5.2481e-13);
//finish     let pos = vec3(1.6810e+06, 1.7953e+06, 2.2084e+06);
//finish     let spread = vec3(4.3278e+09, 9.3046e+09, 6.6121e+09);
//finish     var xyz = val * sqrt(2.0 * PI * spread) * cos(pos * phase + shift) * exp(-phase * phase * spread);
//finish     xyz.x += 9.7470e-14 * sqrt(2.0 * PI * 4.5282e+09) * cos(2.2399e+06 * phase + shift.x) * exp(-4.5282e+09 * phase * phase);
//finish     xyz /= 1.0685e-7;
//finish     return mat3x3(
//finish         vec3(3.2404542, -0.9692660, 0.0556434),
//finish         vec3(-1.5371385, 1.8760108, -0.2040259),
//finish         vec3(-0.4985314, 0.0415560, 1.0572252),
//finish     ) * xyz;
//finish }
//finish fn zgx_film(film_ior: f32, c: f32, thickness: f32, f0: vec3<f32>) -> vec3<f32> {
//finish     // Thinning to nothing, the film's index thins to air's.
//finish     let eta = mix(1.0, film_ior, smoothstep(0.0, 0.03, thickness));
//finish     let sin2 = (1.0 - c * c) / (eta * eta);
//finish     let cos2 = 1.0 - sin2;
//finish     if cos2 < 0.0 {
//finish         return vec3(1.0);
//finish     }
//finish     let c2 = sqrt(cos2);
//finish     let r0 = pow((eta - 1.0) / (eta + 1.0), 2.0);
//finish     let r12 = r0 + (1.0 - r0) * pow(1.0 - c, 5.0);
//finish     let t121 = 1.0 - r12;
//finish     let phi12 = select(0.0, PI, eta < 1.0);
//finish     let phi21 = PI - phi12;
//finish     let root = sqrt(clamp(f0, vec3(0.0), vec3(0.9999)));
//finish     let base_ior = (1.0 + root) / (1.0 - root);
//finish     let r1 = pow((base_ior - eta) / (base_ior + eta), vec3(2.0));
//finish     let r23 = r1 + (1.0 - r1) * pow(1.0 - c2, 5.0);
//finish     let phi23 = select(vec3(0.0), vec3(PI), base_ior < vec3(eta));
//finish     let opd = 2.0 * eta * thickness * c2;
//finish     let phi = phi21 + phi23;
//finish     let r123 = clamp(r12 * r23, vec3(1e-5), vec3(0.9999));
//finish     let root123 = sqrt(r123);
//finish     let rs = t121 * t121 * r23 / (1.0 - r123);
//finish     var reflects = r12 + rs;
//finish     var cm = rs - t121;
//finish     for (var m = 1; m <= 2; m++) {
//finish         cm *= root123;
//finish         reflects += cm * 2.0 * zgx_film_sensitivity(f32(m) * opd, f32(m) * phi);
//finish     }
//finish     return max(reflects, vec3(0.0));
//finish }

// What a fragment's finish makes of it, worked out once: its anisotropy's
// direction across and along, and how strong; its film's reflectance and
// how much; its sheen's colour and roughness, how much of what's under it
// shows seen from here, and its albedo; the normal its surroundings
// reflect about (bent along the anisotropy).
//finish struct Finish {
//finish     t: vec3<f32>,
//finish     b: vec3<f32>,
//finish     aniso: f32,
//finish     film_fresnel: vec3<f32>,
//finish     film: f32,
//finish     sheen: vec3<f32>,
//finish     sheen_rough: f32,
//finish     sheen_under: f32,
//finish     sheen_albedo: f32,
//finish     bent: vec3<f32>,
//finish }
// The finish of a fragment facing `n`, seen along `v`, whose point and
// texture coordinates change across the screen by `dpx`, `dpy`, `duvx` and
// `duvy`, reflecting `f0` facing it (`roughness` its perceptual roughness).
//finish fn zgx_finished(n: vec3<f32>, v: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, duvx: vec2<f32>, duvy: vec2<f32>, f0: vec3<f32>, roughness: f32) -> Finish {
//finish     var f: Finish;
//finish     let n_v = max(dot(n, v), 1e-4);
//finish     // The texture's u axis on the surface (as `bumped` finds it), or any
//finish     // direction across it where the coordinates don't change.
//finish     let det = duvx.x * duvy.y - duvy.x * duvx.y;
//finish     let along = (duvy.y * dpx - duvx.y * dpy) * sign(det);
//finish     var t = along - n * dot(n, along);
//finish     if abs(det) < 1e-12 || dot(t, t) < 1e-20 {
//finish         t = cross(n, select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(n.x) > 0.9));
//finish     }
//finish     t = normalize(t);
//finish     let bt = cross(n, t);
//finish     let turn = layers.aniso.yz;
//finish     f.t = normalize(t * turn.x + bt * turn.y);
//finish     f.b = cross(n, f.t);
//finish     f.aniso = layers.aniso.x;
//finish     // Its reflections bent along the anisotropy, the more the stronger
//finish     // and the smoother it is.
//finish     let bent = normalize(cross(cross(f.b, v), f.b));
//finish     let w = pow(1.0 - f.aniso * (1.0 - roughness), 4.0);
//finish     f.bent = normalize(mix(bent, n, w));
//finish     // No film at all where it has no thickness (as Khronos's viewer has it).
//finish     f.film = select(layers.irid.x, 0.0, layers.irid.z <= 0.0);
//finish     f.film_fresnel = zgx_film(layers.irid.y, n_v, layers.irid.z, f0);
//finish     f.sheen = layers.sheen.rgb;
//finish     f.sheen_rough = layers.sheen.w;
//finish     f.sheen_albedo = zgx_sheen_albedo(n_v, f.sheen_rough);
//finish     f.sheen_under = 1.0 - max(f.sheen.r, max(f.sheen.g, f.sheen.b)) * f.sheen_albedo;
//finish     return f;
//finish }
// `shade` with a finish: its specular anisotropic as it says, its Fresnel
// its film's as much as it says, and a sheen over it all.
//finish fn shade_finished(base: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>, f: Finish) -> vec3<f32> {
//finish     let h = (v + l) / max(length(v + l), 1e-4);
//finish     let n_l = max(dot(n, l), 0.0);
//finish     let n_v = max(dot(n, v), 1e-4);
//finish     let n_h = max(dot(n, h), 0.0);
//finish     let a = roughness * roughness;
//finish //Q     let f0 = mix(vec3(0.04), base, metallic);
//finish //Q     let f90 = vec3(1.0);
//finish //R     let f0 = mix(layers.spec.w * min(layers.spec.rgb, vec3(1.0)), base, metallic);
//finish //R     let f90 = vec3(mix(layers.spec.w, 1.0, metallic));
//finish     let schlick = f0 + (f90 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
//finish     let fresnel = mix(schlick, f.film_fresnel, f.film);
//finish     var specular: vec3<f32>;
//finish     if f.aniso > 0.0 {
//finish         // Anisotropic GGX (Heitz 2014), its visibility height-correlated.
//finish         let at = mix(a, 1.0, f.aniso * f.aniso);
//finish         let ab = a;
//finish         let a2 = at * ab;
//finish         let s = vec3(ab * dot(f.t, h), at * dot(f.b, h), a2 * n_h);
//finish         let w2 = a2 / max(dot(s, s), 1e-12);
//finish         let d = a2 * w2 * w2 / PI;
//finish         let gv = n_l * length(vec3(at * dot(f.t, v), ab * dot(f.b, v), n_v));
//finish         let gl = n_v * length(vec3(at * dot(f.t, l), ab * dot(f.b, l), n_l));
//finish         let vis = clamp(0.5 / max(gv + gl, 1e-6), 0.0, 1.0);
//finish         specular = fresnel * d * vis;
//finish     } else {
//finish         specular = fresnel * ggx(n_h, a) * smith(n_v, n_l, a) / (4.0 * n_v * max(n_l, 1e-4));
//finish     }
//finish     let diffuse = (1.0 - fresnel) * (1.0 - metallic) * base / PI;
//finish     // A sheen takes from what's under it as much as it reflects, seen from
//finish     // here or from the light, whichever is more (as Khronos's viewer does).
//finish     let most = max(f.sheen.r, max(f.sheen.g, f.sheen.b));
//finish     let under = min(f.sheen_under, 1.0 - most * zgx_sheen_albedo(n_l, f.sheen_rough));
//finish     let sheen = f.sheen * zgx_charlie(n_h, f.sheen_rough) * zgx_sheen_visibility(max(n_l, 1e-4), n_v, f.sheen_rough);
//finish     return ((diffuse + specular) * under + sheen) * radiance * n_l;
//finish }
