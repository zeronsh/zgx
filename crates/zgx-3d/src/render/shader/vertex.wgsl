// The mesh's vertex stage. With the depth prepass (`//Z `) it places a
// vertex by `zgx_place`, as the prepass does, its clip position
// `@invariant`; without (`//B `), as it always has.

struct Out {
//B     @builtin(position) position: vec4<f32>,
//Z     @builtin(position) @invariant position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) emissive: vec3<f32>,
    @location(4) material: vec3<f32>,
    @location(5) uv: vec2<f32>,
//U     @location(6) second_uv: vec2<f32>,
//G     @location(7) scale: f32,
//V     @location(8) @interpolate(flat) instance: vec4<f32>,
}

//Z // What a vertex stage reads of a mesh: its vertex's attributes and its
//Z // instance's (the meshes' pass's and the depth prepass's alike).
//Z struct VertexAttributes {
//Z     @location(0) position: vec3<f32>,
//Z     @location(1) normal: vec3<f32>,
//Z     @location(2) uv: vec2<f32>,
//Z     @location(3) m0: vec4<f32>,
//Z     @location(4) m1: vec4<f32>,
//Z     @location(5) m2: vec4<f32>,
//Z     @location(6) m3: vec4<f32>,
//Z     @location(7) n0: vec4<f32>,
//Z     @location(8) n1: vec4<f32>,
//Z     @location(9) n2: vec4<f32>,
//Z     @location(10) color: vec4<f32>,
//Z     @location(11) emissive: vec4<f32>,
//Z //S     @location(12) joints: vec4<u32>,
//Z //S     @location(13) weights: vec4<f32>,
//Z //U     @location(14) second_uv: vec2<f32>,
//Z //C     @location(15) vertex_color: vec4<f32>,
//Z //M     @builtin(vertex_index) index: u32,
//Z //V     @builtin(instance_index) instance_index: u32,
//Z }
//Z // A vertex placed: where it is in the world, its normal there (not
//Z // normalised), and the matrix that placed it (its joints', skinned).
//Z struct Placed {
//Z     world: vec4<f32>,
//Z     normal: vec3<f32>,
//Z     model: mat4x4<f32>,
//Z //V     instance: vec4<f32>,
//Z }
//Z // Where vertex `a` is this frame: morphed, skinned, then moved by a
//Z // custom material's `displace`. The meshes' pass and the prepass both
//Z // place by it, so their depths are the same to the bit (`@invariant`).
//Z fn zgx_place(a: VertexAttributes) -> Placed {
//Z     var model = mat4x4(a.m0, a.m1, a.m2, a.m3);
//Z     var normals = mat3x3(a.n0.xyz, a.n1.xyz, a.n2.xyz);
//Z     var at = a.position;
//Z     var facing = a.normal;
//Z     var joints_from = u32(a.emissive.w);
//Z //M     let morphed = morph(joints_from, a.index, a.position, a.normal);
//Z //M     at = morphed.position;
//Z //M     facing = morphed.normal;
//Z //M     joints_from = morphed.joints;
//Z //S     model = skinned(joints_from, a.joints, a.weights);
//Z //S     normals = mat3x3(model[0].xyz, model[1].xyz, model[2].xyz);
//Z //V     var zv: VertexIn;
//Z //V     zv.position = at;
//Z //V     zv.normal = facing;
//Z //V     zv.uv = a.uv;
//Z //V     zv.uv1 = a.uv;
//Z //V //U     zv.uv1 = a.second_uv;
//Z //V     zv.color = vec4(1.0);
//Z //V //C     zv.color = a.vertex_color;
//Z //V     zv.world = (model * vec4(at, 1.0)).xyz;
//Z //V     zv.instance = vec4(0.0);
//Z //V //mesh_shader     zv.instance = zgx_instances[a.instance_index];
//Z //V     let zd = zgx_displace(zv);
//Z //V     at = zd.position;
//Z //V     facing = zd.normal;
//Z //O     return Placed(model * vec4(at, 1.0), normals * facing, model);
//Z //V     return Placed(model * vec4(at, 1.0), normals * facing, model, zv.instance);
//Z }
//Z @vertex fn vertex(a: VertexAttributes) -> Out {
//Z     let p = zgx_place(a);
//Z     var tint = a.color;
//Z //C     tint *= a.vertex_color;
//Z     return Out(
//Z         frame.view_projection * p.world, p.world.xyz, p.normal, tint, a.emissive.xyz, vec3(a.n0.w, a.n1.w, a.n2.w), a.uv,
//Z //U         a.second_uv,
//Z //G         length(p.model[0].xyz),
//Z //V         p.instance,
//Z     );
//Z }
//B @vertex fn vertex(
//B     @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
//B     @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>,
//B     @location(7) n0: vec4<f32>, @location(8) n1: vec4<f32>, @location(9) n2: vec4<f32>,
//B     @location(10) color: vec4<f32>, @location(11) emissive: vec4<f32>,
//B //S     @location(12) joints: vec4<u32>, @location(13) weights: vec4<f32>,
//B //U     @location(14) second_uv: vec2<f32>,
//B //C     @location(15) vertex_color: vec4<f32>,
//B //M     @builtin(vertex_index) index: u32,
//B //V     @builtin(instance_index) instance_index: u32,
//B ) -> Out {
//B     var model = mat4x4(m0, m1, m2, m3);
//B     var normals = mat3x3(n0.xyz, n1.xyz, n2.xyz);
//B     var at = position;
//B     var facing = normal;
//B     var joints_from = u32(emissive.w);
//B //M     let morphed = morph(joints_from, index, position, normal);
//B //M     at = morphed.position;
//B //M     facing = morphed.normal;
//B //M     joints_from = morphed.joints;
//B //S     model = skinned(joints_from, joints, weights);
//B //S     normals = mat3x3(model[0].xyz, model[1].xyz, model[2].xyz);
//B //V     var zv: VertexIn;
//B //V     zv.position = at;
//B //V     zv.normal = facing;
//B //V     zv.uv = uv;
//B //V     zv.uv1 = uv;
//B //V //U     zv.uv1 = second_uv;
//B //V     zv.color = vec4(1.0);
//B //V //C     zv.color = vertex_color;
//B //V     zv.world = (model * vec4(at, 1.0)).xyz;
//B //V     zv.instance = vec4(0.0);
//B //V //mesh_shader     zv.instance = zgx_instances[instance_index];
//B //V     let zd = zgx_displace(zv);
//B //V     at = zd.position;
//B //V     facing = zd.normal;
//B     let world = model * vec4(at, 1.0);
//B     let n = normals * facing;
//B     var tint = color;
//B //C     tint *= vertex_color;
//B     return Out(
//B         frame.view_projection * world, world.xyz, n, tint, emissive.xyz, vec3(n0.w, n1.w, n2.w), uv,
//B //U         second_uv,
//B //G         length(model[0].xyz),
//B //V         zv.instance,
//B     );
//B }
