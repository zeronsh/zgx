// Sparkles: four-point stars twinkling in and past the box. params: [color,
// size], [count, life, margin, _], [_], [seed, intensity, reduced, _].
fn fx_star(q: vec2<f32>, size: f32, scale: f32) -> f32 {
    // A concave four-pointed star, its centre white hot.
    let a = abs(q) / max(size, 0.01);
    let d = pow(pow(a.x + 0.0001, 0.6) + pow(a.y + 0.0001, 0.6), 1.0 / 0.6) - 1.0;
    return clamp(0.5 - d * size * scale, 0.0, 1.0);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let count = i32(clamp(input.params[1].x, 0.0, 12.0));
    let life = max(input.params[1].y, 0.1);
    let margin = input.params[1].z;
    let size = input.params[0].a;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    var light = 0.0;
    var hot = 0.0;
    for (var i = 0; i < count; i += 1) {
        let n = u32(i);
        let span = life * (1.0 + fx_rand(n, seed + 7u));
        let offset = fx_rand(n, seed + 11u) * span;
        let t = (input.time + offset) / span;
        let cycle = floor(t);
        var phase = t - cycle;
        if reduced {
            phase = 0.35;
        }
        let key = n * 97u + u32(select(cycle, 0.0, reduced)) * 131u + seed;
        let at = vec2(fx_rand(key, 1u), fx_rand(key, 2u)) * (input.size + vec2(2.0 * margin)) - vec2(margin);
        // Sizes lean small, with a few large.
        let big = fx_rand(key, 3u);
        let radius = size * (0.45 + 0.9 * big * big);
        // Skip what it can't reach: a star costs its neighbours nothing.
        if length(input.p - at) > radius * 1.5 {
            continue;
        }
        // Pops in, overshooting, and shrinks away; rests between.
        let grow = smoothstep(0.0, 0.15, phase) * (1.0 + 0.3 * exp(-pow((phase - 0.15) / 0.1, 2.0)));
        let shrink = 1.0 - smoothstep(0.55, 0.95, phase);
        let twinkle = 0.75 + 0.25 * sin(input.time * (5.0 + 4.0 * big) * FX_TAU * 0.5 + big * 40.0);
        let spin = (phase - 0.5) * 0.8 + big * 3.0;
        let c = cos(spin);
        let s = sin(spin);
        let q = input.p - at;
        let turned = vec2(c * q.x - s * q.y, s * q.x + c * q.y);
        let star = fx_star(turned, radius * grow * shrink + 0.001, input.scale) * twinkle;
        light = max(light, star);
        hot = max(hot, star * exp(-dot(q, q) / max(radius * radius * 0.05, 0.01)));
    }
    let color = mix(input.params[0].rgb, vec3(1.0), clamp(hot, 0.0, 1.0));
    return fx_over(vec4(color, clamp(light * input.params[3].y, 0.0, 1.0)), input.color);
}
