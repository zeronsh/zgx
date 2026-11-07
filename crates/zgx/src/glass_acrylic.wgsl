// The material a glass box draws on Windows where the system has none
// (`crate::glass`): Windows' Acrylic, made as WinUI's `AcrylicBrush` makes
// it. What's behind it, blurred already by the box's backdrop blur (a
// sigma of 30), takes the tint's luminosity at the luminosity opacity
// (the luminosity blend), then the tint's hue and saturation at the tint
// opacity (the colour blend); a grey noise lies over that at 0.02. With
// Transparency effects off it's its fallback colour alone. params:
// [tint, tint opacity], [luminosity opacity, fallback weight, _, _],
// [fallback], [luminosity colour].
fn zgx_acrylic_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// The blend modes' luminosity (W3C compositing's non-separable modes).
fn zgx_acrylic_lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.3, 0.59, 0.11));
}
// `c` with luminosity `l`, its colour clipped into range as W3C's does.
fn zgx_acrylic_set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    let moved = c + (l - zgx_acrylic_lum(c));
    let lum = zgx_acrylic_lum(moved);
    let lo = min(min(moved.r, moved.g), moved.b);
    let hi = max(max(moved.r, moved.g), moved.b);
    var out = moved;
    if lo < 0.0 {
        out = lum + (out - lum) * lum / max(lum - lo, 0.00001);
    }
    if hi > 1.0 {
        out = lum + (out - lum) * (1.0 - lum) / max(hi - lum, 0.00001);
    }
    return out;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let d = zgx_acrylic_box(input.p, input.size, input.radii);
    let tint = input.params[0];
    let options = input.params[1];
    let behind = zgx_backdrop(input.p).rgb;
    // Luminosity: the backdrop's hue and saturation, the tint's lightness.
    var color = mix(behind, zgx_acrylic_set_lum(behind, zgx_acrylic_lum(input.params[3].rgb)), options.x);
    // Colour: the tint's hue and saturation, that lightness.
    color = mix(color, zgx_acrylic_set_lum(tint.rgb, zgx_acrylic_lum(color)), tint.a);
    // Its noise: a grey tile of mean 0.5 and spread 0.21, per pixel.
    let pixel = floor(input.p * input.scale);
    let n = fract(sin(dot(pixel, vec2(12.9898, 78.233))) * 43758.5453);
    color = mix(color, vec3(0.5 + (n - 0.5) * 0.72), 0.02);
    color = mix(color, input.params[2].rgb, options.y);
    let glass = vec4(clamp(color, vec3(0.0), vec3(1.0)), clamp(0.5 - d * input.scale, 0.0, 1.0));
    let a = input.color.a + glass.a * (1.0 - input.color.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((input.color.rgb * input.color.a + glass.rgb * glass.a * (1.0 - input.color.a)) / a, a);
}
