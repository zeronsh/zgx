// Electric: lightning crawling along the box's edge. params: [color,
// amplitude], [frequency, speed, core, glow], [strands, flicker, _, _], [seed,
// intensity, reduced, _].
fn fx_strand(s: f32, strike: f32, seed: u32, freq: f32) -> f32 {
    return (fx_noise(vec2(s * freq, strike * 1.7), seed) - 0.5) * 1.3
        + (fx_noise(vec2(s * freq * 2.3, strike * 2.9), seed + 5u) - 0.5) * 0.55
        + (fx_noise(vec2(s * freq * 5.1, strike * 7.3), seed + 9u) - 0.5) * 0.25;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let amplitude = input.params[0].a;
    let freq = max(input.params[1].x, 0.01);
    let speed = input.params[1].y;
    let core = max(input.params[1].z, 0.2);
    let halo = max(input.params[1].w, 0.5);
    let strands = i32(clamp(input.params[2].x, 1.0, 3.0));
    let flicker = input.params[2].y;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    let d = fx_box(input.p, input.size, input.radii);
    // A coordinate running round the box's edge, in points.
    let q = input.p - input.size * 0.5;
    let perimeter = (input.size.x + input.size.y) * 2.0;
    let s = fract(atan2(q.y, q.x) / FX_TAU + 0.5) * perimeter;
    let t = select(input.time * speed, 2.0, reduced);
    var light = 0.0;
    var hot = 0.0;
    for (var i = 0; i < strands; i += 1) {
        // Each strikes, holds, and strikes again near the same channel.
        let strike = floor(t + f32(i) * 0.37);
        let hold = fract(t + f32(i) * 0.37);
        let n = fx_strand(s, strike + t * 0.15, seed + u32(i) * 23u, freq);
        let off = abs(d - n * amplitude);
        let fade = select(1.0 - flicker * smoothstep(0.1, 1.0, hold), 0.6, reduced);
        let thin = max(core * (1.0 - 0.3 * f32(i)), 0.2);
        hot = max(hot, fade * exp(-pow(off / thin, 2.0)));
        light = max(light, fade * (0.55 * exp(-pow(off / halo, 2.0)) + 0.25 * exp(-off / (halo * 3.0))));
    }
    // It seams where the coordinate wraps: closed there.
    let seam = smoothstep(0.0, perimeter * 0.015, min(s, perimeter - s));
    let intensity = input.params[3].y;
    let glow = vec4(input.params[0].rgb, clamp(light * intensity, 0.0, 1.0));
    let bolt = vec4(mix(input.params[0].rgb, vec3(1.0), 0.85), clamp(hot * intensity, 0.0, 1.0) * mix(0.4, 1.0, seam));
    return fx_over(bolt, fx_over(input.color, glow));
}
