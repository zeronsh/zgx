// The frame's uniforms and the scene's bindings (group 0).

struct Frame {
    view_projection: mat4x4<f32>,
    camera: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    points: array<array<vec4<f32>, 3>, 8>,
    params: vec4<f32>,
    shadow: mat4x4<f32>,
    shadow_texel: vec4<f32>,
    environment: vec4<f32>,
    irradiance: array<vec4<f32>, 9>,
    look: vec4<f32>,
//V     view: mat4x4<f32>,
//V     projection: mat4x4<f32>,
//V     size: vec2<f32>,
//V //F     time: f32,
//V //F     delta: f32,
//V //F     index: u32,
//V //J     zgx_time: f32,
//V //J     zgx_delta: f32,
//V //J     zgx_index: u32,
//V     layers: ZgxLayers,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
//E @group(0) @binding(3) var environment_cube: texture_cube<f32>;
//E @group(0) @binding(4) var environment_sampler: sampler;
