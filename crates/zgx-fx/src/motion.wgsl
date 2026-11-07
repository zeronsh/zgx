// Motion blur: the content smeared along its velocity. params: [dx, dy,
// samples, _] in points over the shutter, [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let by = input.params[0].xy * input.params[3].y;
    let samples = i32(clamp(input.params[0].z, 2.0, 32.0));
    var sum = vec4(0.0);
    var total = 0.0;
    for (var i = 0; i < samples; i += 1) {
        let u = (f32(i) + 0.5) / f32(samples);
        // A raised-cosine shutter: its ends soft.
        let w = 0.5 - 0.5 * cos(u * FX_TAU);
        let c = zgx_sample(input.p - by * (u - 0.5));
        sum += vec4(c.rgb * c.a, c.a) * w;
        total += w;
    }
    let m = sum / max(total, 1e-5);
    return select(vec4(0.0), vec4(m.rgb / m.a, m.a), m.a > 0.0);
}
