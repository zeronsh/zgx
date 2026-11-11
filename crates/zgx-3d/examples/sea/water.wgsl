#zgx material 1
// The sea's surface (examples/sea.rs): moved and shaded by the wave field a
// compute pass writes (`waves.wgsl`), read where each vertex and pixel is,
// the tile repeating. Not animated itself: a sea whose waves are still
// draws nothing more.
//
// Parameters:
//   field   the waves: height, and its slopes along x and z
//   deep    the water body's colour (linear RGBA)
//   tile    the field's tile, in metres

// The field's texel `i`, the tile wrapping.
fn texel(i: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(params.field));
    return textureLoad(params.field, ((i % size) + size) % size, 0).xyz;
}

// The waves at `p` (x, z): height, and its slopes, between texels.
fn waves(p: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(params.field));
    let at = p / params.tile * size - 0.5;
    let base = floor(at);
    let f = at - base;
    let i = vec2<i32>(base);
    let below = mix(texel(i), texel(i + vec2(1, 0)), f.x);
    let above = mix(texel(i + vec2(0, 1)), texel(i + vec2(1, 1)), f.x);
    return mix(below, above, f.y);
}

fn displace(v: VertexIn) -> Displaced {
    var d: Displaced;
    let h = waves(v.world.xz);
    d.position = v.position + vec3(0.0, h.x, 0.0);
    d.normal = normalize(vec3(-h.y, 1.0, -h.z));
    return d;
}

fn surface(s: SurfaceIn) -> Surface {
    var m = surface_default(s);
    let h = waves(s.world_position.xz);
    m.normal = normalize(vec3(-h.y, 1.0, -h.z));
    m.base_color = params.deep.rgb;
    m.metallic = 0.0;
    m.roughness = 0.04;
    return m;
}
