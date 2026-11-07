// Fluted glass: ribs each a little cylinder lens over what's behind the box.
// params: [rib, angle, refraction, dispersion], [frost, highlight, _, _], [_],
// [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let rib = max(input.params[0].x, 1.0);
    let angle = input.params[0].y;
    let bend = input.params[0].z * input.params[3].y;
    let spread = input.params[0].w;
    let highlight = input.params[1].y * input.params[3].y;
    let across = vec2(cos(angle), sin(angle));
    let s = dot(input.p, across) / rib;
    let f = fract(s) - 0.5;
    // The strip under it comes back squeezed and flipped.
    let off = -2.0 * f * bend;
    let r = zgx_backdrop(input.p + across * off * (1.0 + spread)).r;
    let g = zgx_backdrop(input.p + across * off).g;
    let b = zgx_backdrop(input.p + across * off * (1.0 - spread)).b;
    var color = vec3(r, g, b);
    // A sheen on each rib, a shaded groove between.
    color += highlight * (0.14 * exp(-pow((f + 0.18) / 0.14, 2.0)) - 0.18 * exp(-pow((abs(f) - 0.5) / 0.06, 2.0)));
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(input.color, vec4(clamp(color, vec3(0.0), vec3(1.0)), cover));
}
