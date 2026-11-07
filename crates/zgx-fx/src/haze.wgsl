// Heat haze: what's behind the box wavers, rising. params: [amount, scale,
// speed, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let amount = input.params[0].x * input.params[3].y;
    let scale = max(input.params[0].y, 1.0);
    let t = select(input.time * input.params[0].z, 4.0, input.params[3].z > 0.5);
    let seed = fx_seed(input.params);
    let q = input.p / scale + vec2(0.0, t);
    let wave = vec2(fx_fbm(q, seed) - 0.5, fx_fbm(q + vec2(5.2, 1.3), seed + 3u) - 0.5);
    let d = fx_box(input.p, input.size, input.radii);
    let edge = smoothstep(0.0, 18.0, -d);
    let behind = zgx_backdrop(input.p + wave * amount * 2.0 * edge);
    return fx_over(input.color, behind);
}
