#zgx material 1
// Wet: a surface layer over every lit mesh while it rains. Rain darkens
// what drinks it and smooths what it films, most where it lands (faces
// turned up) and a little down the sides; on the ground, puddles gather
// where the noise of the surface says it's low, mirror-smooth and flat.
// Already-smooth surfaces (water, glass, polished metal) are left alone.
//
// Parameters:
//   wet      0 to 1: how wet everything is
//   ground   the ground's height (m): puddles form there and just above

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    if params.wet <= 0.0 || m.roughness < 0.1 {
        return m;
    }
    let up = clamp(s.geometric_normal.y, 0.0, 1.0);
    let w = params.wet * (0.35 + 0.65 * smoothstep(0.3, 0.9, up));
    // Paint and concrete darken as their pores fill; metal less so.
    m.base_color *= mix(1.0, mix(0.55, 0.85, m.metallic), w);
    m.roughness = mix(m.roughness, 0.14, w);
    // Puddles, on the ground alone.
    let ground = 1.0 - smoothstep(0.05, 0.4, abs(s.world_position.y - params.ground));
    if up > 0.97 && ground > 0.0 {
        let p = s.world_position.xz;
        let low = noise2(p * 0.11) * 0.7 + noise2(p * 0.43) * 0.3;
        let puddle = smoothstep(0.58, 0.64, low) * params.wet * ground;
        m.roughness = mix(m.roughness, 0.02, puddle);
        m.base_color *= mix(1.0, 0.6, puddle);
        m.normal = normalize(mix(m.normal, vec3(0.0, 1.0, 0.0), puddle));
    }
    return m;
}
