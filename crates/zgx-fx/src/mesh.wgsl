// A mesh gradient: three colours' points drifting, blended by distance.
// Rows: [a, drift], [b, sharpness], [c, speed], [seed, intensity, reduced, warp].
fn fill(input: FillInput) -> vec4<f32> {
    let drift = input.params[0].a;
    let sharp = max(input.params[1].a, 0.1);
    let t = fx_fill_time(input) * input.params[2].a;
    let seed = fx_seed(input.params);
    let uv = input.p / max(input.size, vec2(1.0));
    let warp = input.params[3].w;
    let w = vec2(fx_noise(uv * 2.2 + vec2(t * 0.5, 3.0), seed), fx_noise(uv * 2.2 + vec2(1.7, t * 0.5), seed + 5u)) - 0.5;
    let at = uv + w * warp;
    var sum = vec3(0.0);
    var total = 0.0;
    for (var i = 0; i < 5; i += 1) {
        let n = u32(i);
        let phase = fx_rand(n, seed) * FX_TAU;
        let home = vec2(fx_rand(n, seed + 11u), fx_rand(n, seed + 13u));
        let point = home + drift * vec2(cos(t + phase), sin(t * 0.8 + phase * 1.3));
        let d = max(length(at - point), 0.16);
        let weight = pow(1.0 / d, sharp * 2.0);
        let color = select(select(input.params[2].rgb, input.params[1].rgb, i % 3 == 1), input.params[0].rgb, i % 3 == 0);
        sum += color * weight;
        total += weight;
    }
    // Fine dither against banding on a long ramp.
    let grain = (fx_cell(vec2<i32>(input.p * input.scale), seed) - 0.5) / 255.0;
    return vec4(sum / total + vec3(grain), input.params[3].y);
}
