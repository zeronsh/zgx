// Shockwave: a ring bending what's behind the box as it travels from where
// it was fired. params: [amount, reach, width, _], then a shot a row: [x, y,
// age, strength] (age < 0 with motion reduced).
fn fx_wave(input: LayerInput, shot: vec4<f32>, amount: f32, reach: f32, width: f32) -> vec3<f32> {
    if shot.w <= 0.0 || shot.z < 0.0 {
        return vec3(0.0);
    }
    let life = clamp(shot.z, 0.0, 1.0);
    let r = (1.0 - (1.0 - life) * (1.0 - life)) * reach;
    let q = input.p - shot.xy;
    let l = max(length(q), 0.001);
    let s = (l - r) / max(width * (0.5 + life), 0.5);
    // Its profile: out, then in, smooth at both ends.
    let bend = s * exp(-s * s) * (1.0 - life) * (1.0 - life) * amount * shot.w * 2.3;
    return vec3(q / l * bend, exp(-s * s) * (1.0 - life) * shot.w);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let amount = input.params[0].x;
    let reach = max(input.params[0].y, 1.0);
    let width = input.params[0].z;
    var by = vec2(0.0);
    var lit = 0.0;
    var soft = 0.0;
    for (var i = 1; i < 4; i += 1) {
        let wave = fx_wave(input, input.params[i], amount, reach, width);
        by += wave.xy;
        lit += wave.z;
        if input.params[i].z < 0.0 {
            // Reduced motion: a soft brightening, no travel.
            soft += 0.2 * input.params[i].w * sin(clamp(-input.params[i].z, 0.0, 1.0) * 3.14159);
        }
    }
    // Bending to nothing at the box's edge, so no seam shows there.
    let settle = smoothstep(0.0, 24.0, -fx_box(input.p, input.size, input.radii));
    by *= settle;
    lit *= settle;
    let behind = zgx_backdrop(input.p + by);
    let lifted = vec4(behind.rgb + vec3(0.08 * lit + soft), behind.a);
    return fx_over(input.color, clamp(lifted, vec4(0.0), vec4(1.0)));
}
