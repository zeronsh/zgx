// Pulse: rings or a halo from the box's edge. params: [color, reach],
// [count, period, width, style], [lag, _, _, _], [seed, intensity, reduced, _].
fn fx_beat(phase: f32) -> f32 {
    // Lub-dub: two quick swells in each period.
    let a = exp(-pow((phase - 0.12) / 0.05, 2.0));
    let b = 0.7 * exp(-pow((phase - 0.3) / 0.07, 2.0));
    return a + b;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let reach = max(input.params[0].a, 1.0);
    let count = max(input.params[1].x, 1.0);
    let period = max(input.params[1].y, 0.05);
    let width = max(input.params[1].z, 0.25);
    let style = input.params[1].w;
    let intensity = input.params[3].y;
    let reduced = input.params[3].z > 0.5;
    let d = fx_box(input.p, input.size, input.radii);
    let outside = max(d, 0.0);
    let t = input.time / period;
    var light = 0.0;
    if reduced {
        // A halo that holds still.
        light = 0.45 * exp(-pow(outside / (0.45 * reach), 2.0));
    } else if style < 0.5 {
        // Radar: `count` rings travelling out, fading as they go.
        for (var i = 0; i < 6; i += 1) {
            if f32(i) >= count {
                break;
            }
            let phase = fract(t + f32(i) / count);
            let ring = phase * reach;
            let fade = (1.0 - phase) * (1.0 - phase);
            light += fade * exp(-pow((d - ring) / width, 2.0));
        }
    } else if style < 1.5 {
        // Breathe: a halo growing and easing back.
        let breath = 0.5 - 0.5 * cos(t * FX_TAU);
        light = (0.2 + 0.7 * breath) * exp(-pow(outside / (0.2 * reach + 0.3 * reach * breath), 2.0));
    } else {
        // Heartbeat: rings on each beat.
        let phase = fract(t);
        let beat = fx_beat(phase);
        light = beat * exp(-pow(outside / (0.15 * reach + 0.5 * reach * phase), 2.0));
    }
    light *= select(0.0, 1.0, d > -1.0);
    let window = 1.0 - smoothstep(reach * 0.9, reach, outside);
    let glow = vec4(input.params[0].rgb, clamp(fx_knee(light * intensity * 1.6) * window, 0.0, 1.0));
    return fx_over(input.color, glow);
}
