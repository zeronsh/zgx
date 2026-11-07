// The material a glass box draws where the system has none (`crate::glass`):
// what's behind it, blurred already by the box's backdrop blur, saturated and
// laid under a colour, bent a little at its rim, with a lit edge. params:
// [bevel, depth, saturation, tint alpha], [tint, rim], [_], [_].
fn zgx_glass_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let d = zgx_glass_box(input.p, input.size, input.radii);
    let inside = clamp(-d / max(input.params[0].x, 0.5), 0.0, 1.0);
    let slope = pow(1.0 - inside, 3.0);
    let eps = 0.75;
    let gradient = vec2(
        zgx_glass_box(input.p + vec2(eps, 0.0), input.size, input.radii) - zgx_glass_box(input.p - vec2(eps, 0.0), input.size, input.radii),
        zgx_glass_box(input.p + vec2(0.0, eps), input.size, input.radii) - zgx_glass_box(input.p - vec2(0.0, eps), input.size, input.radii),
    );
    let dir = -gradient / max(length(gradient), 0.0001);
    let behind = zgx_backdrop(input.p + dir * slope * input.params[0].y);
    let luma = dot(behind.rgb, vec3(0.2126, 0.7152, 0.0722));
    var color = mix(vec3(luma), behind.rgb, input.params[0].z);
    color = mix(clamp(color, vec3(0.0), vec3(1.0)), input.params[1].rgb, input.params[0].w);
    let lit = dot(dir, vec2(-0.5, 0.86));
    color += input.params[1].a * exp(-pow(-d / 1.1, 2.0)) * (0.6 * max(lit, 0.0) + 0.25 * max(-lit, 0.0));
    let glass = vec4(clamp(color, vec3(0.0), vec3(1.0)), clamp(0.5 - d * input.scale, 0.0, 1.0));
    let a = input.color.a + glass.a * (1.0 - input.color.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((input.color.rgb * input.color.a + glass.rgb * glass.a * (1.0 - input.color.a)) / a, a);
}
