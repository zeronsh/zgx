// Clear glass: flat in its middle and at its very edge, what's behind it bent
// in the band between, each colour by its own amount. params: [width, strength,
// dispersion, highlight], [tint, tint alpha], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let width = max(input.params[0].x, 0.5);
    let strength = input.params[0].y * input.params[3].y;
    let spread = input.params[0].z;
    let d = fx_box(input.p, input.size, input.radii);
    let t = clamp(-d / width, 0.0, 1.0);
    // 0 at the boundary and in the core, 1 between.
    let bump = 4.0 * t * (1.0 - t);
    let eps = 0.75;
    let gradient = vec2(
        fx_box(input.p + vec2(eps, 0.0), input.size, input.radii) - fx_box(input.p - vec2(eps, 0.0), input.size, input.radii),
        fx_box(input.p + vec2(0.0, eps), input.size, input.radii) - fx_box(input.p - vec2(0.0, eps), input.size, input.radii),
    );
    let dir = -gradient / max(length(gradient), 0.0001);
    let bend = dir * bump * bump * strength;
    let r = zgx_backdrop(input.p + bend * (1.0 - spread));
    let g = zgx_backdrop(input.p + bend);
    let b = zgx_backdrop(input.p + bend * (1.0 + spread));
    var color = vec3(r.r, g.g, b.b);
    color = mix(color, input.params[1].rgb, input.params[1].a * input.params[3].y);
    // A faint lit edge, for the eye to find it.
    let lit = input.params[0].w * input.params[3].y * exp(-pow(d / 0.9, 2.0)) * (0.55 + 0.45 * -dir.y);
    color = clamp(color + vec3(lit), vec3(0.0), vec3(1.0));
    let glass = vec4(color, max(max(r.a, g.a), b.a) * fx_cover(d, input.scale));
    return fx_over(input.color, glass);
}
