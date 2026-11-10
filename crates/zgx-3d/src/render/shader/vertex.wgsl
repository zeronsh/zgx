// The mesh's vertex stage.

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) emissive: vec3<f32>,
    @location(4) material: vec3<f32>,
    @location(5) uv: vec2<f32>,
//U     @location(6) second_uv: vec2<f32>,
//G     @location(7) scale: f32,
}

@vertex fn vertex(
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) m0: vec4<f32>, @location(4) m1: vec4<f32>, @location(5) m2: vec4<f32>, @location(6) m3: vec4<f32>,
    @location(7) n0: vec4<f32>, @location(8) n1: vec4<f32>, @location(9) n2: vec4<f32>,
    @location(10) color: vec4<f32>, @location(11) emissive: vec4<f32>,
//S     @location(12) joints: vec4<u32>, @location(13) weights: vec4<f32>,
//U     @location(14) second_uv: vec2<f32>,
//C     @location(15) vertex_color: vec4<f32>,
//M     @builtin(vertex_index) index: u32,
) -> Out {
    var model = mat4x4(m0, m1, m2, m3);
    var normals = mat3x3(n0.xyz, n1.xyz, n2.xyz);
    var at = position;
    var facing = normal;
    var joints_from = u32(emissive.w);
//M     let morphed = morph(joints_from, index, position, normal);
//M     at = morphed.position;
//M     facing = morphed.normal;
//M     joints_from = morphed.joints;
//S     model = skinned(joints_from, joints, weights);
//S     normals = mat3x3(model[0].xyz, model[1].xyz, model[2].xyz);
//V     var zv: VertexIn;
//V     zv.position = at;
//V     zv.normal = facing;
//V     zv.uv = uv;
//V     zv.uv1 = uv;
//V //U     zv.uv1 = second_uv;
//V     zv.color = vec4(1.0);
//V //C     zv.color = vertex_color;
//V     zv.world = (model * vec4(at, 1.0)).xyz;
//V     let zd = zgx_displace(zv);
//V     at = zd.position;
//V     facing = zd.normal;
    let world = model * vec4(at, 1.0);
    let n = normals * facing;
    var tint = color;
//C     tint *= vertex_color;
    return Out(
        frame.view_projection * world, world.xyz, n, tint, emissive.xyz, vec3(n0.w, n1.w, n2.w), uv,
//U         second_uv,
//G         length(model[0].xyz),
    );
}
