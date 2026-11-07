// Moulded plastic under one soft light. Rows: [color, roughness], [_, dome],
// [_, bevel], [seed, intensity, reduced, angle].
fn fill(input: FillInput) -> vec4<f32> {
    let color = input.params[0].rgb;
    let roughness = clamp(input.params[0].a, 0.03, 1.0);
    let dome = input.params[1].a;
    let bevel = input.params[2].a;
    let angle = input.params[3].w;
    let n = fx_dome(input, dome, bevel, vec2(0.0));
    let light = normalize(vec3(sin(angle) * 0.6, -cos(angle) * 0.6, 0.8));
    let view = vec3(0.0, 0.0, 1.0);
    let half_way = normalize(light + view);
    let lambert = max(dot(n, light), 0.0);
    let shade = 0.48 + 0.65 * lambert;
    // A microfacet highlight: small and bright when glossy, broad when matte.
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, half_way), 0.0);
    let denom = nh * nh * (a2 - 1.0) + 1.0;
    let spec = min(a2 / (3.14159 * denom * denom) * 0.12, 0.8);
    let fresnel = 0.04 + 0.96 * pow(1.0 - max(n.z, 0.0), 5.0);
    var c = color * min(shade, 1.0);
    if shade > 1.0 {
        c = mix(c, vec3(1.0), (shade - 1.0) * 0.6);
    }
    // They fill what room is left above the base, so it never clips to white.
    c = c + (vec3(1.0) - c) * clamp(spec + fresnel * 0.35, 0.0, 1.0);
    return vec4(c, input.params[3].y);
}
