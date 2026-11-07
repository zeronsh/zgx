// Shimmer: a bright band crossing the content. params: [color, band],
// [period, angle, pause, on_box], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let band = max(input.params[0].a, 1.0);
    let period = max(input.params[1].x, 0.05);
    let angle = input.params[1].y;
    let pause = clamp(input.params[1].z, 0.0, 0.95);
    let intensity = input.params[3].y;
    var strength = 0.0;
    let across = vec2(cos(angle), sin(angle));
    let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
    if input.params[3].z > 0.5 {
        // Reduced motion: a still, faint sheen.
        strength = 0.35;
    } else {
        // The band starts and ends off the box, so it never cuts in.
        let phase = fract(input.time / period);
        let moving = clamp(phase / (1.0 - pause), 0.0, 1.0);
        let at = -band * 2.0 + moving * (extent + band * 4.0);
        let centre = input.size * 0.5;
        let s = dot(input.p - centre, across) + extent * 0.5;
        strength = exp(-pow((s - at) / band, 2.0));
    }
    // Over the box's own shape where asked, otherwise what the content paints.
    let cover = select(input.color.a, fx_cover(fx_box(input.p, input.size, input.radii), input.scale), input.params[1].w > 0.5);
    let sheen = vec4(input.params[0].rgb, clamp(strength * intensity, 0.0, 1.0) * cover);
    return fx_over(sheen, input.color);
}
