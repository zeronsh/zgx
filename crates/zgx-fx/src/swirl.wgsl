// A swirl: a twist about the centre over warped noise, through a colour ramp.
// Rows: [a, scale], [b, speed], [c, twist], [seed, intensity, reduced, warp].
fn fill(input: FillInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = fx_fill_time(input) * input.params[1].a;
    let twist = input.params[2].a;
    let warp = input.params[3].w;
    let seed = fx_seed(input.params);
    let q = input.p - input.size * 0.5;
    let r = length(q) / max(length(input.size * 0.5), 1.0);
    let spin = twist * (1.0 - r) * 3.0 + t * 0.3;
    let c = cos(spin);
    let s = sin(spin);
    let at = vec2(c * q.x - s * q.y, s * q.x + c * q.y) / scale;
    let a = vec2(fx_fbm(at + vec2(t * 0.2, 0.0), seed), fx_fbm(at + vec2(5.2, 1.3 + t * 0.2), seed + 3u));
    let n = fx_fbm(at + a * warp * 2.0, seed + 7u);
    return vec4(fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, smoothstep(0.25, 0.75, n)), input.params[3].y);
}
