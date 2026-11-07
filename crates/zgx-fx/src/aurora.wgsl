// Aurora: curtains of light over the box. params: [color, scale], [second,
// speed], [third, feather], [seed, intensity, reduced, _].
fn fx_curtain(p: vec2<f32>, t: f32, seed: u32, scale: f32, bias: f32) -> f32 {
    let x = p.x / scale;
    // Its foot wanders; rays hang from it, thinning upward.
    let foot = bias + 0.32 * (fx_noise(vec2(x * 0.7 + t * 0.6, t * 0.2), seed) - 0.5);
    let y = p.y / scale;
    let rays = 0.55 + 0.45 * fx_noise(vec2(x * 7.0 + t * 1.2, y * 0.6), seed + 3u);
    let rise = y - foot;
    return rays * exp(-pow(max(rise, 0.0) / 0.42, 2.0)) * smoothstep(-0.14, 0.0, rise);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = select(input.time * input.params[1].a, 3.0, input.params[3].z > 0.5);
    let seed = fx_seed(input.params);
    // Upward from the box's bottom.
    let p = vec2(input.p.x, input.size.y - input.p.y);
    let a = fx_curtain(p, t, seed, scale, 0.3);
    let b = fx_curtain(p + vec2(40.0, 0.0), t * 0.8 + 5.0, seed + 19u, scale, 0.55);
    let c = fx_curtain(p + vec2(90.0, 0.0), t * 1.15 + 11.0, seed + 41u, scale, 0.12);
    let glow = input.params[0].rgb * a + input.params[1].rgb * b + input.params[2].rgb * c;
    let light = clamp(a + b + c, 0.0, 1.0);
    // It feathers out inside the box's shape.
    let feather = max(input.params[2].a, 0.5);
    let d = fx_box(input.p, input.size, input.radii);
    let edge = smoothstep(0.0, feather, -d);
    let alpha = fx_knee(light * 1.3) * edge * input.params[3].y;
    let color = glow / max(max(glow.r, max(glow.g, glow.b)), 0.001);
    return fx_over(vec4(color, clamp(alpha, 0.0, 1.0)), input.color);
}
