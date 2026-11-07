// Fire: flames rising from the box's top edge, with embers. params: [color
// (the tips' tint), height], [scale, speed, embers, warp], [_], [seed,
// intensity, reduced, _]. Temperature falls from white through amber and red.
fn fx_flame(heat: f32, tint: vec3<f32>) -> vec3<f32> {
    let white = vec3(1.0, 0.95, 0.72);
    let amber = vec3(1.0, 0.62, 0.14);
    let a = mix(amber, white, smoothstep(0.62, 0.98, heat));
    return mix(tint, a, smoothstep(0.18, 0.6, heat));
}
fn layer(input: LayerInput) -> vec4<f32> {
    let height = max(input.params[0].a, 1.0);
    let scale = max(input.params[1].x, 1.0);
    let speed = input.params[1].y;
    let warp = input.params[1].w;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    let t = select(input.time * speed, 7.0, reduced);
    // The flame's own height above the top edge, 0 to 1, sides pulled in.
    let up = (-input.p.y) / height;
    let x = input.p.x;
    let edge = min(x, input.size.x - x) / max(min(input.size.x * 0.5, height), 1.0);
    let sides = smoothstep(-0.1, 0.5, edge);
    let q = vec2(x / scale, up * height / scale - t);
    let bent = q + warp * (0.25 + up) * vec2(fx_noise(q * 0.9 + vec2(0.0, t * 0.2), seed) - 0.5, 0.0);
    let n = fx_fbm(bent, seed + 3u);
    let flicker = 0.92 + 0.08 * fx_noise(vec2(x / (scale * 3.0), t * 4.0), seed + 7u);
    let body = n * flicker * (1.0 - clamp(up, 0.0, 1.5) * 0.92) * sides * 1.75 - clamp(up, 0.0, 1.5) * 0.28;
    var heat = clamp(body, 0.0, 1.0);
    heat *= select(0.0, 1.0, input.p.y < 0.5);
    let flame = vec4(fx_flame(heat, input.params[0].rgb), smoothstep(0.08, 0.34, heat));
    // Embers flung up, slowing and cooling.
    var ember = 0.0;
    if !reduced {
        let count = i32(clamp(input.params[1].z, 0.0, 12.0));
        for (var i = 0; i < count; i += 1) {
            let life = 1.1 + 0.8 * fx_rand(u32(i), seed + 31u);
            let u = (input.time * speed * 0.4 + fx_rand(u32(i), seed + 37u) * life) / life;
            let cycle = floor(u);
            let age = u - cycle;
            let key = u32(i) * 53u + u32(cycle) * 17u + seed;
            let start = fx_rand(key, 1u) * input.size.x;
            let rise = (1.0 - exp(-age * 2.2)) * height * 1.2;
            let sway = sin(age * 9.0 + fx_rand(key, 2u) * 6.0) * 5.0;
            let at = vec2(start + sway, -rise);
            let r = 0.8 + 0.8 * fx_rand(key, 3u);
            let d = length(input.p - at);
            ember = max(ember, (1.0 - age) * (1.0 - age) * exp(-pow(d / r, 2.0)));
        }
    }
    let spark = vec4(vec3(1.0, 0.68, 0.28), clamp(ember, 0.0, 1.0));
    let intensity = input.params[3].y;
    var out = fx_over(vec4(spark.rgb, spark.a * intensity), vec4(flame.rgb, flame.a * intensity));
    // Under the content: flames rise from behind it.
    return fx_over(input.color, out);
}
