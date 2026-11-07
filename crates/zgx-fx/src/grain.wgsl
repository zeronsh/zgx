// Grain: fine noise over the content, by device pixel. params: [amount, mono,
// fps, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let seed = fx_seed(input.params);
    let fps = input.params[0].z;
    let frame = select(u32(floor(input.time * fps)), 0u, fps <= 0.0 || input.params[3].z > 0.5);
    let pixel = vec2<i32>(floor(input.p * input.scale));
    let key = seed + frame * 1009u;
    let tri = fx_cell(pixel, key) + fx_cell(pixel, key + 5u) - 1.0;
    let amount = input.params[0].x * input.params[3].y;
    var noise = vec3(tri);
    if input.params[0].y < 0.5 {
        noise = vec3(tri, fx_cell(pixel, key + 11u) + fx_cell(pixel, key + 13u) - 1.0, fx_cell(pixel, key + 17u) + fx_cell(pixel, key + 19u) - 1.0);
    }
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    // Over a clear box it shows as faint speckle of its own.
    let base = fx_over(input.color, vec4(0.5, 0.5, 0.5, amount * cover));
    return vec4(clamp(base.rgb + noise * amount * cover, vec3(0.0), vec3(1.0)), max(base.a, 0.0));
}
