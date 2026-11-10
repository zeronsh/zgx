// Drawing into the sun's shadow map.

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
