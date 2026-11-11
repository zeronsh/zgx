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
