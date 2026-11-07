// Ripple: rings from where it was fired, cut by the box's shape. params:
// [color, reach], then a shot a row: [x, y, age, strength] (age is its life's
// fraction, and below 0 with motion reduced).
fn fx_ring(input: LayerInput, shot: vec4<f32>, reach: f32) -> f32 {
    if shot.w <= 0.0 {
        return 0.0;
    }
    if shot.z < 0.0 {
        // Reduced motion: a soft brightening, no travel.
        return 0.35 * shot.w * sin(clamp(-shot.z, 0.0, 1.0) * 3.14159);
    }
    let life = clamp(shot.z, 0.0, 1.0);
    let eased = 1.0 - (1.0 - life) * (1.0 - life) * (1.0 - life);
    let r = eased * reach;
    let d = length(input.p - shot.xy) - r;
    let width = 3.0 + 18.0 * life;
    let fade = (1.0 - life) * (1.0 - life);
    return shot.w * fade * (exp(-pow(d / width, 2.0)) + 0.3 * smoothstep(0.0, -r - 1.0, d));
}
fn layer(input: LayerInput) -> vec4<f32> {
    let reach = max(input.params[0].a, 1.0);
    var light = 0.0;
    light += fx_ring(input, input.params[1], reach);
    light += fx_ring(input, input.params[2], reach);
    light += fx_ring(input, input.params[3], reach);
    let inside = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(vec4(input.params[0].rgb, clamp(light, 0.0, 1.0) * inside), input.color);
}
