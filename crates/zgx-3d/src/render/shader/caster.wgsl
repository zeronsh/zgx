// Drawing into the sun's shadow map.

@vertex fn shadow_vertex(
    @location(0) position: vec3<f32>,
    @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>,
//P     @location(11) emissive: vec4<f32>,
//S     @location(12) joints: vec4<u32>, @location(13) weights: vec4<f32>,
//M     @builtin(vertex_index) index: u32,
//U     @location(14) second_uv: vec2<f32>,
//K     @location(2) uv: vec2<f32>, @location(9) n2: vec4<f32>,
//V     @location(1) normal: vec3<f32>,
//V //N     @location(2) uv: vec2<f32>,
//V     @builtin(instance_index) instance_index: u32,
//N ) -> @builtin(position) vec4<f32> {
//K ) -> ShadowOut {
    var model = mat4x4(m0, m1, m2, m3);
    var at = position;
//P     var joints_from = u32(emissive.w);
//M     let morphed = morph(joints_from, index, position, vec3(0.0));
//M     at = morphed.position;
//M     joints_from = morphed.joints;
//S     model = skinned(joints_from, joints, weights);
    // A custom material moves its shadow as it moves its mesh.
//V     var zv: VertexIn;
//V     zv.position = at;
//V     zv.normal = normal;
//V     zv.uv = uv;
//V     zv.uv1 = uv;
//V     zv.color = vec4(1.0);
//V     zv.world = (model * vec4(at, 1.0)).xyz;
//V     zv.instance = vec4(0.0);
//V //mesh_shader     zv.instance = zgx_instances[instance_index];
//V     at = zgx_displace(zv).position;
//N     return frame.shadow * model * vec4(at, 1.0);
//K     var cut_uv = uv;
//K //U     cut_uv = placed(0u, uv, second_uv);
//K //O     return ShadowOut(frame.shadow * model * vec4(at, 1.0), cut_uv, n2.w);
//K //V     let zn = normalize(mat3x3(model[0].xyz, model[1].xyz, model[2].xyz) * normal);
//K //V     return ShadowOut(frame.shadow * model * vec4(at, 1.0), cut_uv, n2.w, (model * vec4(at, 1.0)).xyz, zn, zv.instance);
}
//K struct ShadowOut {
//K     @builtin(position) position: vec4<f32>,
//K     @location(0) uv: vec2<f32>,
//K     @location(1) word: f32,
//K //V     @location(2) world: vec3<f32>,
//K //V     @location(3) normal: vec3<f32>,
//K //V     @location(4) @interpolate(flat) instance: vec4<f32>,
//K }
//K // A surface's alpha cutoff from its third material word (`surface_word`).
//K fn cutoff_of(word: f32) -> f32 {
//K     return select(-word, word - 1.0, word > 0.5);
//K }
//K @fragment fn shadow_fragment(in: ShadowOut) {
//K     if textureSample(base_texture, map_sampler, in.uv).a < cutoff_of(in.word) {
//K         discard;
//K     }
//K //D     var zs: SurfaceIn;
//K //D     zs.world_position = in.world;
//K //D     zs.normal = in.normal;
//K //D     zs.geometric_normal = in.normal;
//K //D     zs.uv = in.uv;
//K //D     zs.uv1 = in.uv;
//K //D     zs.color = vec4(1.0);
//K //D     zs.view = -frame.sun_direction.xyz;
//K //D     zs.front_facing = true;
//K //D     zs.instance = in.instance;
//K //D     if zgx_opacity(zs) < zgx_cutoff(in.word) {
//K //D         discard;
//K //D     }
//K }
