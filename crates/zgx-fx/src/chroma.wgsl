// A chromatic split: what's behind the box with its channels pulled apart.
// params: [dx, dy, radial, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let intensity = input.params[3].y;
    let radial = input.params[0].z;
    let q = input.p - input.size * 0.5;
    let reach = max(length(input.size * 0.5), 1.0);
    let by = (input.params[0].xy + q / reach * radial) * intensity;
    let r = zgx_backdrop(input.p + by).r;
    let c = zgx_backdrop(input.p);
    let b = zgx_backdrop(input.p - by).b;
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(input.color, vec4(r, c.g, b, c.a * cover));
}
