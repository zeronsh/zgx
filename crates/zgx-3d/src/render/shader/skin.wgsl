// Joints and morph targets (groups 2 and 3).

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
