// Mapping light to the display (and, between, the environment's light:
// it sits where it did when the shader was one file).

fn neutral(color: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desaturation = 0.15;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = color - offset;
    let peak = max(c.r, max(c.g, c.b));
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3(new_peak), g);
}
//E // Where the environment's light along `d` comes from: the room turned
//E // by its rotation about the vertical.
//E fn turned(d: vec3<f32>) -> vec3<f32> {
//E     let c = frame.environment.y;
//E     let s = frame.environment.z;
//E     return vec3(c * d.x - s * d.z, d.y, s * d.x + c * d.z);
//E }
//E // Its diffuse light over π facing `n` (nine spherical harmonics).
//E fn irradiance(n: vec3<f32>) -> vec3<f32> {
//E     let k = frame.irradiance;
//E     return k[0].rgb + k[1].rgb * n.y + k[2].rgb * n.z + k[3].rgb * n.x
//E         + k[4].rgb * (n.x * n.y) + k[5].rgb * (n.y * n.z) + k[6].rgb * (3.0 * n.z * n.z - 1.0)
//E         + k[7].rgb * (n.x * n.z) + k[8].rgb * (n.x * n.x - n.y * n.y);
//E }
//E // The split sum's second factor, the specular's scale and bias by
//E // angle and roughness: Karis's fit to its table (no texture).
//E fn reflectance(f0: vec3<f32>, roughness: f32, n_v: f32) -> vec3<f32> {
//E     let r = roughness * vec4(-1.0, -0.0275, -0.572, 0.022) + vec4(1.0, 0.0425, 1.04, -0.04);
//E     let a004 = min(r.x * r.x, exp2(-9.28 * n_v)) * r.x + r.y;
//E     let ab = vec2(-1.04, 1.04) * a004 + r.zw;
//E     return f0 * ab.x + ab.y;
//E }
// ACES's filmic curve, Stephen Hill's fit: into its working space, its
// reference transform and display transform as one rational curve, back.
// Exposed by 1 / 0.6 first, as three.js does, so mid grey stays about mid.
fn aces(color: vec3<f32>) -> vec3<f32> {
    let into = mat3x3(vec3(0.59719, 0.07600, 0.02840), vec3(0.35458, 0.90834, 0.13383), vec3(0.04823, 0.01566, 0.83777));
    let back = mat3x3(vec3(1.60475, -0.10208, -0.00327), vec3(-0.53108, 1.10813, -0.07276), vec3(-0.07367, -0.00605, 1.07602));
    let v = into * (color / 0.6);
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return clamp(back * (a / b), vec3(0.0), vec3(1.0));
}
fn tone_map(color: vec3<f32>) -> vec3<f32> {
    if frame.look.x > 1.5 {
        return color;
    }
    if frame.look.x > 0.5 {
        return aces(color);
    }
    return neutral(color);
}
fn encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}
//G fn decode(c: vec3<f32>) -> vec3<f32> {
//G     let low = c / 12.92;
//G     let high = pow((c + 0.055) / 1.055, vec3(2.4));
//G     return select(high, low, c <= vec3(0.04045));
//G }
