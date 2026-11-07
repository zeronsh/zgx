// Waves: layered bands with two-sine crests, each shading the one behind.
// Rows: [a, amplitude], [b, frequency], [c, speed], [seed, intensity, reduced, softness].
fn fill(input: FillInput) -> vec4<f32> {
    let amplitude = input.params[0].a;
    let frequency = input.params[1].a;
    let t = fx_fill_time(input) * input.params[2].a;
    let soft = max(input.params[3].w, 0.0005);
    let seed = fx_seed(input.params);
    let uv = input.p / max(input.size, vec2(1.0));
    var color = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, 0.0);
    for (var i = 0; i < 5; i += 1) {
        let k = f32(i) / 4.0;
        let n = u32(i);
        let phase = fx_rand(n, seed) * FX_TAU;
        let line = 0.15 + 0.75 * k
            + amplitude * sin(uv.x * frequency * FX_TAU + t * (1.0 + k) + phase)
            + amplitude * 0.5 * sin(uv.x * frequency * 2.3 * FX_TAU - t * 0.7 + phase * 2.0);
        let band = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, k);
        let under = smoothstep(line - soft, line + soft, uv.y);
        // Darker just under each crest.
        let shade = 1.0 - 0.2 * exp(-pow((uv.y - line) / 0.04, 2.0));
        color = mix(color, band * shade, under);
    }
    return vec4(color, input.params[3].y);
}
