// A cathode-ray tube's look over what's behind the box: bulged, scanned and
// vignetted. params: [curve, scanlines, mask, split], [_], [_], [seed,
// intensity, reduced, fps-less flicker].
fn layer(input: LayerInput) -> vec4<f32> {
    let curve = input.params[0].x * input.params[3].y;
    let lines = input.params[0].y * input.params[3].y;
    let mask = input.params[0].z * input.params[3].y;
    let split = input.params[0].w * input.params[3].y;
    let uv = input.p / max(input.size, vec2(1.0)) * 2.0 - 1.0;
    let r2 = dot(uv, uv);
    let bent = (uv * (1.0 + curve * (r2 - 1.0)) + 1.0) * 0.5 * input.size;
    let r = zgx_backdrop(bent + vec2(split, 0.0)).r;
    let g = zgx_backdrop(bent).g;
    let b = zgx_backdrop(bent - vec2(split, 0.0)).b;
    var color = vec3(r, g, b);
    // Scan lines, a phosphor mask, a vignette; a line per 2 points at 2x.
    let line = 0.5 + 0.5 * cos(bent.y * FX_TAU / 3.0);
    color *= 1.0 - lines * (1.0 - line) * 0.55;
    let phosphor = fract(bent.x / 3.0);
    let tint = vec3(step(phosphor, 0.33), step(0.33, phosphor) * step(phosphor, 0.66), step(0.66, phosphor));
    color *= mix(vec3(1.0), 0.72 + tint * 0.5, mask);
    color *= 1.0 - 0.35 * input.params[3].y * smoothstep(0.5, 2.0, r2);
    // Past the curved glass: dark.
    let off = any(bent < vec2(0.0)) || any(bent > input.size);
    let d = fx_box(input.p, input.size, input.radii);
    let screen = vec4(select(clamp(color, vec3(0.0), vec3(1.0)), vec3(0.02), off), fx_cover(d, input.scale));
    return fx_over(input.color, screen);
}
