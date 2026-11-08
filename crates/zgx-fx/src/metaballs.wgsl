// Metaballs: blobs merging, an antialiased edge and a glossy rim. Rows: [a,
// size], [b, speed], [back, gloss], [seed, intensity, reduced, count].
fn fill(input: FillInput) -> vec4<f32> {
    let size = input.params[0].a;
    let t = fx_fill_time(input) * input.params[1].a;
    let gloss = input.params[2].a;
    let seed = fx_seed(input.params);
    let count = i32(clamp(input.params[3].w, 1.0, 8.0));
    let unit = min(input.size.x, input.size.y);
    let aspect = input.size / unit;
    let uv = input.p / unit;
    var field = 0.0;
    var mixed = 0.0;
    for (var i = 0; i < count; i += 1) {
        let n = u32(i);
        let phase = vec2(fx_rand(n, seed + 3u), fx_rand(n, seed + 5u)) * FX_TAU;
        let rate = vec2(0.6 + fx_rand(n, seed + 7u), 0.5 + fx_rand(n, seed + 9u));
        let at = (0.5 + 0.42 * vec2(sin(t * rate.x + phase.x), cos(t * rate.y + phase.y))) * aspect;
        let radius = size * (0.7 + 0.6 * fx_rand(n, seed + 11u));
        let d = uv - at;
        let w = radius * radius / max(dot(d, d), 0.00001);
        field += w;
        mixed += w * fx_rand(n, seed + 13u);
    }
    // Its edge a pixel wide, from the slope of the field held under 1: as the
    // field is at the edge, where it crosses 0, and flat where the field
    // climbs without bound at a blob's centre (its own slope there would
    // mix the background into pixels around it).
    let held = 1.0 - 1.0 / max(field, 0.0001);
    let slope = max(length(vec2(dpdx(held), dpdy(held))), 0.0001);
    let edge = clamp(held / slope + 0.5, 0.0, 1.0);
    let body = mix(input.params[0].rgb, input.params[1].rgb, clamp(mixed / max(field, 0.001), 0.0, 1.0));
    let rim = exp(-pow((field - 1.25) / 0.25, 2.0)) * gloss;
    let color = mix(input.params[2].rgb, body + vec3(rim * 0.6), edge);
    return vec4(color, input.params[3].y);
}
