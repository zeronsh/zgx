// A ripple through what's painted, from where it's fired: a wave travelling at
// a finite speed and dying exponentially, each pixel read along its radius
// (`amplitude sin(frequency t) exp(-decay t)`, t delayed by the distance over
// the speed). params: [amplitude, frequency, decay, speed], then a shot a row:
// [x, y, age (of its 2 second life; below 0 with motion reduced), strength].
const FX_WAVE_LIFE: f32 = 2.0;
fn fx_wave_at(input: LayerInput, shot: vec4<f32>) -> vec3<f32> {
    if shot.w <= 0.0 || shot.z < 0.0 {
        return vec3(0.0);
    }
    let q = input.p - shot.xy;
    let l = max(length(q), 0.001);
    let t = max(shot.z * FX_WAVE_LIFE - l / max(input.params[0].w, 1.0), 0.0);
    let height = sin(input.params[0].y * t) * exp(-input.params[0].z * t) * shot.w;
    return vec3(q / l * height * input.params[0].x, height);
}
fn layer(input: LayerInput) -> vec4<f32> {
    var by = vec2(0.0);
    var height = 0.0;
    var soft = 0.0;
    for (var i = 1; i < 4; i += 1) {
        let wave = fx_wave_at(input, input.params[i]);
        by += wave.xy;
        height += wave.z;
        if input.params[i].z < 0.0 {
            // Reduced motion: a soft brightening, no travel.
            soft += 0.2 * input.params[i].w * sin(clamp(-input.params[i].z, 0.0, 1.0) * 3.14159);
        }
    }
    let c = zgx_sample(input.p + by);
    return vec4(clamp(c.rgb + vec3(0.25 * height + soft) * c.a, vec3(0.0), vec3(1.0)), c.a);
}
