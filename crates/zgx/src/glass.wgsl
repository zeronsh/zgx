// The material a glass box draws where the system has none (`crate::glass`),
// made as the system's glass makes its face, in Display P3 as a Mac's display
// does: what's behind it, blurred already by the box's backdrop blur, its
// luma moved into a range with its colour kept, a fill laid over, a tint, and
// a lit rim a point wide. params:
// [black, white, saturation, top], [fill, fill alpha],
// [rim gain, rim luma, rim lift, shadow alpha], [tint, tint alpha].
fn zgx_glass_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
fn zgx_glass_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}
// Encoded sRGB to encoded Display P3 (the same curve, wider primaries), and
// back, clipped to sRGB.
fn zgx_glass_decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}
fn zgx_glass_encode(c: vec3<f32>) -> vec3<f32> {
    let l = clamp(c, vec3(0.0), vec3(1.0));
    return select(1.055 * pow(l, vec3(1.0 / 2.4)) - 0.055, l * 12.92, l <= vec3(0.0031308));
}
fn zgx_glass_p3(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3(vec3(0.8225, 0.0332, 0.0171), vec3(0.1774, 0.9669, 0.0724), vec3(0.0, 0.0, 0.9108));
    return zgx_glass_encode(m * zgx_glass_decode(c));
}
fn zgx_glass_srgb(c: vec3<f32>) -> vec3<f32> {
    let m = mat3x3(vec3(1.2249, -0.0420, -0.0197), vec3(-0.2247, 1.0419, -0.0786), vec3(0.0, 0.0, 1.0979));
    return zgx_glass_encode(m * zgx_glass_decode(c));
}
// How much of the box's own shadow (`GlassStyle::drawn_shadow`: 8 points
// down, a sigma of 20) lies under `p`, as the renderer draws it.
fn zgx_glass_shadow(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let x = zgx_glass_box(p - vec2(0.0, 8.0), size, radii) / (20.0 * 1.41421356);
    let x2 = x * x;
    let e = sign(x) * sqrt(1.0 - exp(-x2 * (1.27323954 + 0.147 * x2) / (1.0 + 0.147 * x2)));
    return 0.5 - 0.5 * e;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let d = zgx_glass_box(input.p, input.size, input.radii);
    let rim = input.params[2];
    let behind = zgx_glass_p3(zgx_backdrop(input.p).rgb);
    let face = input.params[0];
    let luma = zgx_glass_luma(behind);
    var color = clamp(vec3(face.x + (face.y - face.x) * luma) + face.z * (behind - luma), vec3(0.0), vec3(face.w));
    color = mix(color, input.params[1].rgb, input.params[1].a);
    let tint = input.params[3];
    if tint.a > 0.0 {
        // A tint takes the face's lightness, as the system's tint layer
        // does (its colour matrices fitted: light where the rim lifts far),
        // its components the display's as they are.
        let k = select(vec4(0.735, 0.256, -0.074, 0.082), vec4(1.008, -0.094, -0.016, 0.121), rim.z < 0.5);
        let y = zgx_glass_luma(color);
        let tinted = tint.rgb * (k.x + k.y * y) + vec3(k.z + k.w * y);
        color = mix(color, clamp(tinted, vec3(0.0), vec3(1.0)), tint.a);
    }
    // The rim: the point inside the edge, the face lifted and its colour
    // pushed out, over it at 0.7.
    let lifted = clamp(rim.x * color - rim.y * zgx_glass_luma(color) + rim.z, vec3(0.0), vec3(1.0));
    color = mix(color, lifted, 0.7 * clamp((0.8 + d) * input.scale + 0.5, 0.0, 1.0));
    let glass = vec4(zgx_glass_srgb(color), clamp(0.5 - d * input.scale, 0.0, 1.0));
    // What the box holds over it, its own shadow under that taken out: the
    // renderer paints a box's shadow under all of it.
    let shadow = rim.w * zgx_glass_shadow(input.p, input.size, input.radii);
    let held = input.color.rgb * input.color.a;
    let over = clamp((input.color.a - shadow) / (1.0 - shadow), 0.0, 1.0);
    let a = over + glass.a * (1.0 - over);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((held + glass.rgb * glass.a * (1.0 - over)) / a, a);
}
