// Voronoi: orbiting cells with seams exactly between them. Rows: [a, scale],
// [b, speed], [seam, edge], [seed, intensity, reduced, depth].
fn fx_site(c: vec2<i32>, seed: u32, t: f32) -> vec2<f32> {
    let a = fx_cell(c, seed) * FX_TAU;
    let home = vec2(fx_cell(c, seed + 3u), fx_cell(c, seed + 5u));
    return vec2<f32>(c) + 0.5 + (home - 0.5) * 0.6 + 0.2 * vec2(cos(t + a), sin(t + a));
}
fn fill(input: FillInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = fx_fill_time(input) * input.params[1].a * FX_TAU;
    let edge = input.params[2].a;
    let depth = input.params[3].w;
    let seed = fx_seed(input.params);
    let q = input.p / scale;
    let c = vec2<i32>(floor(q));
    var best = 1e9;
    var site = vec2(0.0);
    var key = c;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let at = c + vec2(x, y);
            let s = fx_site(at, seed, t);
            let d = dot(q - s, q - s);
            if d < best {
                best = d;
                site = s;
                key = at;
            }
        }
    }
    // The distance to the nearest seam: along the line to each neighbour.
    var seam = 1e9;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let at = c + vec2(x, y);
            if at.x == key.x && at.y == key.y {
                continue;
            }
            let s = fx_site(at, seed, t);
            let mid = (site + s) * 0.5;
            let dir = normalize(s - site);
            seam = min(seam, dot(mid - q, dir));
        }
    }
    let shade = 1.0 - depth * clamp(sqrt(best), 0.0, 1.0);
    let body = mix(input.params[0].rgb, input.params[1].rgb, fx_cell(key, seed + 9u)) * shade;
    let line = clamp(0.5 + (edge * 0.5 - seam * scale) * input.scale, 0.0, 1.0);
    return vec4(mix(body, input.params[2].rgb, line), input.params[3].y);
}
