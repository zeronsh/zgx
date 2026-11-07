// Liquid metal: a polished surface, rippling, reflecting a studio. Rows:
// [tint, flow], [_, scale], [_, speed], [seed, intensity, reduced, bevel].
fn fill(input: FillInput) -> vec4<f32> {
    let flow = input.params[0].a;
    let scale = max(input.params[1].a, 1.0);
    let t = fx_fill_time(input) * input.params[2].a;
    let seed = fx_seed(input.params);
    let q = input.p / scale;
    var ripple = vec2(0.0);
    if flow > 0.0 {
        let warp = vec2(fx_fbm(q + vec2(t, -t * 0.6), seed), fx_fbm(q + vec2(4.1 - t * 0.7, 2.3 + t * 0.4), seed + 3u));
        let e = 0.05;
        let at = q + warp * 1.2;
        let h = fx_fbm(at + vec2(t * 0.4, 0.0), seed + 7u);
        let hx = fx_fbm(at + vec2(e + t * 0.4, 0.0), seed + 7u);
        let hy = fx_fbm(at + vec2(t * 0.4, e), seed + 7u);
        ripple = vec2(hx - h, hy - h) / e * flow * 0.22;
    }
    let n = fx_dome(input, 0.55, input.params[3].w, ripple);
    let light = fx_studio(n);
    // A conductor's reflection: the tint at normal incidence, white at grazing.
    let fresnel = pow(1.0 - n.z, 5.0);
    let tint = input.params[0].rgb;
    let body = mix(tint * 0.2, mix(tint, vec3(1.0), 0.15), clamp(light, 0.0, 1.0));
    let spec = clamp(light - 0.9, 0.0, 1.0) * 2.4;
    let color = mix(body, vec3(1.0), clamp(fresnel + spec, 0.0, 1.0));
    return vec4(color, input.params[3].y);
}
