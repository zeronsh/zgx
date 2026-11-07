// Halftone: round dots on a rotated screen, each sized by a tone. Rows:
// [ink, cell], [base, angle], [_, kind], [seed, intensity, reduced, _].
fn fx_tone(input: FillInput, at: vec2<f32>, kind: f32) -> f32 {
    let q = at - input.size * 0.5;
    if kind < 0.5 {
        return length(q) / max(length(input.size * 0.5), 1.0);
    }
    return at.y / max(input.size.y, 1.0);
}
fn fill(input: FillInput) -> vec4<f32> {
    let cell = max(input.params[0].a, 2.0);
    let angle = input.params[1].a;
    let c = cos(angle);
    let s = sin(angle);
    let to = mat2x2(vec2(c, s), vec2(-s, c));
    let from_screen = mat2x2(vec2(c, -s), vec2(s, c));
    let q = from_screen * input.p / cell;
    let i = floor(q);
    var ink = 0.0;
    // The four nearest dots, so grown dots merge.
    for (var y = 0; y < 2; y += 1) {
        for (var x = 0; x < 2; x += 1) {
            let centre = i + vec2(f32(x), f32(y)) + vec2(select(-0.5, 0.5, fract(q.x) > 0.5), select(-0.5, 0.5, fract(q.y) > 0.5));
            let at = to * centre * cell;
            let tone = clamp(1.0 - fx_tone(input, at, input.params[2].a), 0.0, 1.0);
            let radius = sqrt(tone) * 0.72;
            let d = (length(q - centre) - radius) * cell;
            ink = max(ink, clamp(0.5 - d * input.scale, 0.0, 1.0));
        }
    }
    return vec4(mix(input.params[1].rgb, input.params[0].rgb, ink), input.params[3].y);
}
