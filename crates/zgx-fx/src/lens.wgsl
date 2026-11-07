// A lens of glass: what's behind it bent at its rim, its colours split there,
// frosted if asked, with a highlight and a shadow. params: [bevel, depth,
// dispersion, magnification], [frost, saturation, lift, tint alpha], [tint,
// rim], [seed, intensity, reduced, shadow].
fn fx_read(p: vec2<f32>, dir: vec2<f32>, bend: f32, spread: f32) -> vec3<f32> {
    let r = zgx_backdrop(p + dir * bend * (1.0 - spread)).r;
    let g = zgx_backdrop(p + dir * bend).g;
    let b = zgx_backdrop(p + dir * bend * (1.0 + spread)).b;
    return vec3(r, g, b);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let bevel = max(input.params[0].x, 0.5);
    let depth = input.params[0].y;
    let spread = input.params[0].z;
    let magnify = input.params[0].w;
    let saturation = input.params[1].y;
    let lift = input.params[1].z;
    let presence = input.params[3].y;
    let d = fx_box(input.p, input.size, input.radii);
    if d > 0.0 {
        // Its shadow past the box.
        let shadow = input.params[3].w * presence * exp(-pow(d / 12.0, 2.0));
        return fx_over(input.color, vec4(0.0, 0.0, 0.0, clamp(shadow, 0.0, 1.0)));
    }
    let centre = input.size * 0.5;
    let inside = clamp(-d / bevel, 0.0, 1.0);
    // A rounded bevel: flat over the middle, steepening toward the rim.
    let slope = pow(1.0 - inside, 3.0);
    // Inward, across the bevel, from the box's shape.
    let eps = 0.75;
    let gradient = vec2(
        fx_box(input.p + vec2(eps, 0.0), input.size, input.radii) - fx_box(input.p - vec2(eps, 0.0), input.size, input.radii),
        fx_box(input.p + vec2(0.0, eps), input.size, input.radii) - fx_box(input.p - vec2(0.0, eps), input.size, input.radii),
    );
    let dir = -gradient / max(length(gradient), 0.0001);
    let bend = slope * depth * presence;
    let at = centre + (input.p - centre) / max(mix(1.0, magnify, presence), 0.01);
    var color = fx_read(at, dir, bend, spread * slope);
    // Saturated and lifted as glass over a window is: more in the dark than
    // in the light. (What's frosted is blurred already: a backdrop blur under
    // this, `Fx::frost`.)
    color = fx_saturate(color, mix(1.0, saturation, presence));
    color = 1.0 - (1.0 - color) * (1.0 - lift * presence);
    color = mix(color, input.params[2].rgb, input.params[1].w * presence);
    // A highlight along the upper lit rim, a counter-line under it.
    let lit = dot(dir, vec2(-0.5, 0.86));
    let rim = input.params[2].a * presence * exp(-pow(-d / 1.1, 2.0));
    color += rim * (0.6 * max(lit, 0.0) + 0.25 * max(-lit, 0.0));
    let glass = vec4(clamp(color, vec3(0.0), vec3(1.0)), fx_cover(d, input.scale));
    return fx_over(input.color, glass);
}
