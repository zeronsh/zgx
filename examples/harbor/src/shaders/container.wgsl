#zgx material 1
// A container's paint, worn as its own life wore it: each container's four
// instance numbers (`World::set_instance_floats`) say how much rust runs
// from its rails, how much grime climbs from the ground, how far its paint
// has faded, and a seed placing its rust and dents.
//
//   instance.x  rust, 0 to 1
//   instance.y  grime, 0 to 1
//   instance.z  fading, 0 to 1 (toward a chalky grey)
//   instance.w  its seed
//
// The shared wear texture and its colour come through `surface_default`.

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let rust = s.instance.x;
    let grime = s.instance.y;
    let fade = s.instance.z;
    let seed = s.instance.w;
    // Faded paint: lighter, greyer and chalkier.
    let grey = dot(m.base_color, vec3(0.2126, 0.7152, 0.0722));
    m.base_color = mix(m.base_color, vec3(grey * 1.15 + 0.03), fade * 0.6);
    m.roughness = mix(m.roughness, 0.85, fade * 0.5);
    // Grime from the ground: up its side by the texture's V (0 at the top).
    let low = smoothstep(0.35, 1.0, s.uv.y);
    let blot = noise2(s.uv * vec2(18.0, 6.0) + seed * 17.0);
    let g = grime * low * (0.5 + blot);
    m.base_color *= 1.0 - 0.45 * clamp(g, 0.0, 1.0);
    // Rust: streaks running down from the top rail, placed by the seed, and
    // patches where the paint's gone.
    let column = noise2(vec2(s.uv.x * 60.0 + seed * 31.0, 0.5));
    let streak = smoothstep(0.62, 0.8, column) * (1.0 - smoothstep(0.1, 0.7, s.uv.y));
    let spots = smoothstep(0.7, 0.85, noise2(s.uv * vec2(40.0, 14.0) + seed * 7.0));
    let r = clamp(rust * (streak + spots), 0.0, 1.0);
    m.base_color = mix(m.base_color, vec3(0.22, 0.08, 0.03), r * 0.8);
    m.roughness = mix(m.roughness, 0.9, r);
    m.metallic = mix(m.metallic, 0.0, r);
    return m;
}
