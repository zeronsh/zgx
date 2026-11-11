// Custom materials' inputs (`#zgx material 1`, docs/3d.md): what an app's
// `displace`, `surface` and `opacity` read and return, and the helpers it
// may call. A pipeline without custom code (`X`) has none of it.

//V struct VertexIn {
//V     position: vec3<f32>,
//V     normal: vec3<f32>,
//V     uv: vec2<f32>,
//V     uv1: vec2<f32>,
//V     color: vec4<f32>,
//V     world: vec3<f32>,
//V     instance: vec4<f32>,
//V }
//V struct Displaced {
//V     position: vec3<f32>,
//V     normal: vec3<f32>,
//V }
//V struct SurfaceIn {
//V     world_position: vec3<f32>,
//V     normal: vec3<f32>,
//V     geometric_normal: vec3<f32>,
//V     uv: vec2<f32>,
//V     uv1: vec2<f32>,
//V     color: vec4<f32>,
//V     view: vec3<f32>,
//V     depth: f32,
//V     screen_uv: vec2<f32>,
//V     front_facing: bool,
//V     instance: vec4<f32>,
//V }
//V struct Surface {
//V     base_color: vec3<f32>,
//V     metallic: f32,
//V     roughness: f32,
//V     normal: vec3<f32>,
//V     emissive: vec3<f32>,
//V     occlusion: f32,
//V     opacity: f32,
//V }
//V // The surface so far: the mesh's material and maps', then each
//V // shader's in turn (`zgx_surface`).
//V var<private> zgx_so_far: Surface;
//V fn surface_default(s: SurfaceIn) -> Surface {
//V     return zgx_so_far;
//V }
//V // A cut-out's threshold from the third material word: its alpha cutoff,
//V // else a half.
//V fn zgx_cutoff(word: f32) -> f32 {
//V     let c = select(-word, word - 1.0, word > 0.5);
//V     return select(c, 0.5, c <= 0.0);
//V }
//V fn zgx_decode(c: vec3<f32>) -> vec3<f32> {
//V     return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
//V }
//V // Jarzynski and Olano's PCG3D: three well-mixed words of three.
//V fn zgx_pcg(v: vec3<u32>) -> vec3<u32> {
//V     var w = v * 1664525u + 1013904223u;
//V     w.x += w.y * w.z;
//V     w.y += w.z * w.x;
//V     w.z += w.x * w.y;
//V     w ^= w >> vec3(16u);
//V     w.x += w.y * w.z;
//V     w.y += w.z * w.x;
//V     w.z += w.x * w.y;
//V     return w;
//V }
//V // A number in [0, 1) for each point, unrelated to its neighbours'.
//V fn hash(p: vec3<f32>) -> f32 {
//V     return f32(zgx_pcg(bitcast<vec3<u32>>(p)).x >> 8u) / 16777216.0;
//V }
//V fn zgx_lattice(c: vec3<f32>) -> f32 {
//V     return f32(zgx_pcg(vec3<u32>(vec3<i32>(c))).x >> 8u) / 16777216.0;
//V }
//V // Smooth value noise in [0, 1], one cell a unit.
//V fn noise3(p: vec3<f32>) -> f32 {
//V     let i = floor(p);
//V     let f = p - i;
//V     let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
//V     let a = mix(zgx_lattice(i), zgx_lattice(i + vec3(1.0, 0.0, 0.0)), u.x);
//V     let b = mix(zgx_lattice(i + vec3(0.0, 1.0, 0.0)), zgx_lattice(i + vec3(1.0, 1.0, 0.0)), u.x);
//V     let c = mix(zgx_lattice(i + vec3(0.0, 0.0, 1.0)), zgx_lattice(i + vec3(1.0, 0.0, 1.0)), u.x);
//V     let d = mix(zgx_lattice(i + vec3(0.0, 1.0, 1.0)), zgx_lattice(i + vec3(1.0)), u.x);
//V     return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
//V }
//V fn noise2(p: vec2<f32>) -> f32 {
//V     return noise3(vec3(p, 0.0));
//V }
//V // A texture sampled bicubically (a B-spline, four bilinear taps) at its
//V // largest level: a mask magnified far shows smooth curves, not
//V // bilinear's diamonds. (`s`: `material_sampler`, filtering linearly.)
//V fn sample_bicubic(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec4<f32> {
//V     let size = vec2<f32>(textureDimensions(t, 0));
//V     let p = uv * size - 0.5;
//V     let f = fract(p);
//V     let i = p - f;
//V     let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
//V     let w1 = (3.0 * f * f * f - 6.0 * f * f + 4.0) / 6.0;
//V     let w2 = (-3.0 * f * f * f + 3.0 * f * f + 3.0 * f + 1.0) / 6.0;
//V     let w3 = f * f * f / 6.0;
//V     let g0 = w0 + w1;
//V     let g1 = w2 + w3;
//V     let lo = (i + 0.5 + (w1 / g0 - 1.0)) / size;
//V     let hi = (i + 0.5 + (w3 / g1 + 1.0)) / size;
//V     let a = textureSampleLevel(t, s, lo, 0.0) * g0.x + textureSampleLevel(t, s, vec2(hi.x, lo.y), 0.0) * g1.x;
//V     let b = textureSampleLevel(t, s, vec2(lo.x, hi.y), 0.0) * g0.x + textureSampleLevel(t, s, hi, 0.0) * g1.x;
//V     return a * g0.y + b * g1.y;
//V }
//V // How much each axis's plane maps a surface facing `n`: its normal's
//V // parts raised to `sharpness` (higher, a crisper seam), summing to 1.
//V fn triplanar_weights(n: vec3<f32>, sharpness: f32) -> vec3<f32> {
//V     let w = pow(abs(n), vec3(sharpness));
//V     return w / max(w.x + w.y + w.z, 1e-6);
//V }
//V // A texture mapped onto a surface by its world position `p` from the
//V // three axes' planes, `scale` repeats a metre, blended by its normal
//V // `n`: no texture coordinates needed, no stretching on steep faces.
//V fn sample_triplanar(t: texture_2d<f32>, s: sampler, p: vec3<f32>, n: vec3<f32>, scale: f32) -> vec4<f32> {
//V     let w = triplanar_weights(n, 4.0);
//V     return textureSample(t, s, p.zy * scale) * w.x
//V         + textureSample(t, s, p.xz * scale) * w.y
//V         + textureSample(t, s, p.xy * scale) * w.z;
//V }
//V // The normal `n` bent by a height `h` (metres, above the surface) as it
//V // changes across the screen with the point `p` (Mikkelsen's surface
//V // gradient): procedural bumps, cracks and joints with no tangents or
//V // texture. Call it where every fragment of a mesh does (not under a
//V // branch that differs between neighbours).
//V fn bump(n: vec3<f32>, p: vec3<f32>, h: f32) -> vec3<f32> {
//V     let dpx = dpdx(p);
//V     let dpy = dpdy(p);
//V     let r1 = cross(dpy, n);
//V     let r2 = cross(n, dpx);
//V     let det = dot(dpx, r1);
//V     let gradient = sign(det) * (dpdx(h) * r1 + dpdy(h) * r2);
//V     return normalize(abs(det) * n - gradient);
//V }
