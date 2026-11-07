// Glow: light around the box. params: [color, radius], [second, style],
// [third, inner], [seed, intensity, reduced, breathe].
fn layer(input: LayerInput) -> vec4<f32> {
    let radius = max(input.params[0].a, 0.5);
    let style = input.params[1].a;
    let intensity = input.params[3].y;
    let d = fx_box(input.p, input.size, input.radii);
    let t = select(input.time, 0.0, input.params[3].z > 0.5);
    let breath = 1.0 - input.params[3].w * (0.5 + 0.5 * cos(t * FX_TAU / 2.6));
    let outside = max(d, 0.0);
    let tight = exp(-pow(outside / (0.35 * radius), 2.0));
    let wide = exp(-pow(outside / (1.2 * radius), 2.0));
    // It ends before the bleed does, so no quad edge shows.
    let window = 1.0 - smoothstep(radius * 2.0, radius * 2.5, outside);
    var light = fx_knee((0.9 * tight + 0.45 * wide) * intensity * breath) * window;
    light *= select(0.0, 1.0, d > -1.0);
    var color = input.params[0].rgb;
    if style > 1.5 {
        // Aura: three colours flowing round the box.
        let q = input.p - input.size * 0.5;
        let turn = atan2(q.y, q.x) / FX_TAU + 0.5;
        let s = fract(turn + t * 0.12 + 0.08 * fx_noise(q * 0.02 + vec2(t * 0.1), fx_seed(input.params)));
        let k = s * 3.0;
        let a = mix(input.params[0].rgb, input.params[1].rgb, smoothstep(0.0, 1.0, k));
        let b = mix(input.params[1].rgb, input.params[2].rgb, smoothstep(1.0, 2.0, k));
        color = select(b, a, k < 1.0);
        if k >= 2.0 {
            color = mix(input.params[2].rgb, input.params[0].rgb, smoothstep(2.0, 3.0, k));
        }
    }
    let inner = input.params[2].a;
    var glow = vec4(color, light);
    if inner > 0.0 && d <= 0.0 {
        glow = fx_over(vec4(color, inner * intensity * exp(-pow(-d / (0.5 * radius), 2.0))), glow);
    }
    var out = fx_over(input.color, glow);
    if style > 0.5 && style < 1.5 {
        // Neon: a white-hot hairline along the edge, over the content's rim.
        let hair = exp(-pow(d / max(0.75, radius * 0.06), 2.0)) * intensity * breath;
        out = fx_over(vec4(mix(color, vec3(1.0), 0.8), clamp(hair, 0.0, 1.0)), out);
    }
    return out;
}
