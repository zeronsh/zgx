// A grainy gradient: noise scatters the ramp, so its colours stipple into
// each other. Rows: [a, amount], [b, kind], [c, angle], [seed, intensity, reduced, fps].
fn fill(input: FillInput) -> vec4<f32> {
    let amount = input.params[0].a;
    let kind = input.params[1].a;
    let angle = input.params[2].a;
    let seed = fx_seed(input.params) + u32(floor(input.time * input.params[3].w)) * 1009u;
    let q = input.p - input.size * 0.5;
    var t = 0.0;
    if kind < 0.5 {
        let across = vec2(cos(angle), sin(angle));
        let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
        t = dot(q, across) / max(extent, 1.0) + 0.5;
    } else if kind < 1.5 {
        t = length(q) / max(length(input.size * 0.5), 1.0);
    } else {
        // Conic: its ramp there and back, so it loops.
        let turn = fract(atan2(q.y, q.x) / FX_TAU + angle / FX_TAU + 1.0);
        t = 1.0 - abs(turn * 2.0 - 1.0);
    }
    let pixel = vec2<i32>(input.p * input.scale);
    let tri = fx_cell(pixel, seed) + fx_cell(pixel, seed + 7u) - 1.0;
    let color = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, t + tri * amount * 0.4);
    return vec4(color + vec3(tri * amount * 0.05), input.params[3].y);
}
