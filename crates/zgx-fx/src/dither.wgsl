// Dither: a tone quantized between two colours in square pixels by a Bayer
// or white-noise pattern. Rows: [a, pixel], [b, angle], [_, pattern], [seed,
// intensity, reduced, speed].
fn fx_bayer(c: vec2<i32>) -> f32 {
    // Recursive 8x8 Bayer, by bits.
    let x = u32(c.x) & 7u;
    let y = u32(c.y) & 7u;
    let v = x ^ y;
    let bits = ((v & 1u) << 5u) | ((y & 1u) << 4u) | ((v & 2u) << 2u) | ((y & 2u) << 1u) | ((v & 4u) >> 1u) | ((y & 4u) >> 2u);
    return (f32(bits) + 0.5) / 64.0;
}
fn fill(input: FillInput) -> vec4<f32> {
    let pixel = max(input.params[0].a, 1.0);
    let angle = input.params[1].a;
    let seed = fx_seed(input.params);
    let t = fx_fill_time(input) * input.params[3].w;
    let cell = floor(input.p / pixel);
    let centre = (cell + 0.5) * pixel;
    let across = vec2(cos(angle), sin(angle));
    let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
    var tone = dot(centre - input.size * 0.5, across) / max(extent, 1.0) + 0.5;
    tone += 0.25 * (fx_noise(centre / 60.0 + vec2(t, -t * 0.7), seed) - 0.5);
    let c = vec2<i32>(cell);
    let threshold = select(fx_cell(c, seed), fx_bayer(c), input.params[2].a < 0.5);
    let color = select(input.params[0].rgb, input.params[1].rgb, tone > threshold);
    return vec4(color, input.params[3].y);
}
