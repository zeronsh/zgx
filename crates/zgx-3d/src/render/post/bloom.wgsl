// Bloom's steps as dispatches of one compute pass (where the device runs
// compute): the same taps as `post.wgsl`'s passes (`bloom_maths.wgsl`,
// prepended), each step's texels written where a pass would draw them. A
// step down reads the one before (the scene for the first); a step up adds
// the next one's tent to this one's down, into a chain of its own (a
// dispatch can't read and write one level).
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
@group(0) @binding(2) var out: texture_storage_2d<rgba16float, write>;
// A step up's own step down.
@group(0) @binding(3) var base: texture_2d<f32>;

// The texel `id` of what's written, if it's one; its centre's place.
fn uv_of(id: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(id) + 0.5) / vec2<f32>(textureDimensions(out));
}
@compute @workgroup_size(8, 8) fn first(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(out)) {
        return;
    }
    textureStore(out, id.xy, vec4(boxes(source, linear, uv_of(id.xy), true), 1.0));
}
@compute @workgroup_size(8, 8) fn down(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(out)) {
        return;
    }
    textureStore(out, id.xy, vec4(boxes(source, linear, uv_of(id.xy), false), 1.0));
}
@compute @workgroup_size(8, 8) fn up(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(out)) {
        return;
    }
    let own = textureLoad(base, vec2<i32>(id.xy), 0).rgb;
    textureStore(out, id.xy, vec4(own + tent(source, linear, uv_of(id.xy)), 1.0));
}
