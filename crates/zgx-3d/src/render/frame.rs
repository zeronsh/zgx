//! The frame's uniforms (camera, lights, shadow, environment, look) and
//! the binding holding them.
use crate::{
    math::Mat4,
    world::{Inner, Light, POINT_LIGHTS},
};
use zgx_gpu::wgpu;
/// Per frame (see the shader's `Frame`).
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct FrameUniforms {
    pub(super) view_projection: [[f32; 4]; 4],
    pub(super) camera: [f32; 4],
    pub(super) sun_direction: [f32; 4],
    pub(super) sun_color: [f32; 4],
    pub(super) sky: [f32; 4],
    pub(super) ground: [f32; 4],
    pub(super) points: [[[f32; 4]; 3]; POINT_LIGHTS],
    /// Point lights in use, whether to encode sRGB itself, exposure,
    /// whether the sun casts shadows.
    pub(super) params: [f32; 4],
    /// The sun's view of the scene, onto the shadow map.
    pub(super) shadow: [[f32; 4]; 4],
    /// A shadow map texel's size in the scene, and in the map (1 / side);
    /// a lookup's offset off a surface facing the sun, and how much less it
    /// is the more it faces it.
    pub(super) shadow_texel: [f32; 4],
    /// The environment's intensity, its rotation's cosine and sine, and its
    /// cube's last level (zeros without one).
    pub(super) environment: [f32; 4],
    /// Its diffuse light (`Prefiltered::irradiance`), a coefficient a row.
    pub(super) irradiance: [[f32; 4]; 9],
    /// The tone mapping: 0 neutral, 1 ACES, 2 none.
    pub(super) look: [f32; 4],
}

pub(super) fn frame_uniforms(world: &Inner, view_projection: Mat4, encode: bool) -> FrameUniforms {
    let mut uniforms = FrameUniforms {
        view_projection: view_projection.columns,
        camera: [
            world.camera.position.x,
            world.camera.position.y,
            world.camera.position.z,
            1.,
        ],
        sun_direction: [0., -1., 0., 0.],
        sun_color: [0.; 4],
        sky: [0.; 4],
        ground: [0.; 4],
        points: [[[0.; 4]; 3]; POINT_LIGHTS],
        params: [0., if encode { 1. } else { 0. }, 1., 0.],
        shadow: Mat4::IDENTITY.columns,
        shadow_texel: [0.; 4],
        environment: [0.; 4],
        irradiance: [[0.; 4]; 9],
        look: [
            match world.tone_mapping {
                crate::ToneMapping::Neutral => 0.,
                crate::ToneMapping::Aces => 1.,
                crate::ToneMapping::Linear => 2.,
            },
            0.,
            0.,
            0.,
        ],
    };
    uniforms.params[2] = world.exposure.exp2();
    if let Some(environment) = &world.environment {
        let (sin, cos) = environment.rotation.sin_cos();
        uniforms.environment = [
            environment.intensity,
            cos,
            sin,
            (crate::environment::LEVELS - 1) as f32,
        ];
        for (row, k) in uniforms
            .irradiance
            .iter_mut()
            .zip(&environment.data.irradiance)
        {
            *row = [k[0], k[1], k[2], 0.];
        }
    }
    // The point lights nearest the camera's target, nearest first, past the
    // eight a frame shades with (ties in the order they were added).
    let target = world.camera.target;
    // Nearest first, in place: `shaded` of them so far.
    let mut points = [(f32::INFINITY, [[0.; 4]; 3]); POINT_LIGHTS];
    let mut shaded = 0;
    world.for_each_light(|light, spot| {
        match light {
            // The first sun shades; others add to it as one.
            Light::Directional {
                direction,
                color,
                intensity,
            } => {
                let d = direction.normalized();
                uniforms.sun_direction = [d.x, d.y, d.z, 0.];
                for (sum, c) in uniforms.sun_color.iter_mut().zip(color) {
                    *sum += c * intensity;
                }
            }
            Light::Hemisphere {
                sky,
                ground,
                intensity,
            } => {
                for (sum, c) in uniforms.sky.iter_mut().zip(sky) {
                    *sum += c * intensity;
                }
                for (sum, c) in uniforms.ground.iter_mut().zip(ground) {
                    *sum += c * intensity;
                }
            }
            Light::Point {
                position,
                color,
                intensity,
                range,
            } => {
                // One off (no lumens, or black) takes no lit one's place.
                if intensity <= 0. || color.iter().all(|c| *c <= 0.) {
                    return;
                }
                let distance = (position - target).length();
                let at = points[..shaded]
                    .iter()
                    .position(|(nearer, _)| distance.total_cmp(nearer).is_lt())
                    .unwrap_or(shaded);
                if at < POINT_LIGHTS {
                    // A spot's cone as glTF fades it: (cos θ · scale +
                    // offset)², θ from its direction; all round, 1.
                    let cone = spot.map_or([0., 0., 0., 1.], |spot| {
                        let (inner, outer) = (spot.inner.cos(), spot.outer.cos());
                        let scale = 1. / (inner - outer).max(1e-3);
                        let d = spot.direction.normalized() * scale;
                        [d.x, d.y, d.z, -outer * scale]
                    });
                    let slot = [
                        [position.x, position.y, position.z, range.max(1e-3)],
                        [color[0], color[1], color[2], intensity],
                        cone,
                    ];
                    // The farthest falls off the end when they're all taken.
                    points.copy_within(at..POINT_LIGHTS - 1, at + 1);
                    points[at] = (distance, slot);
                    shaded = (shaded + 1).min(POINT_LIGHTS);
                }
            }
        }
    });
    for (slot, (_, light)) in uniforms.points.iter_mut().zip(&points[..shaded]) {
        *slot = *light;
    }
    uniforms.params[0] = shaded as f32;
    uniforms
}

/// The main pass's frame: the uniforms, the shadow map (or stand-in), and
/// an environment-lit frame's cube and its sampler.
pub(super) fn frame_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    map: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    cube: Option<(&wgpu::TextureView, &wgpu::Sampler)>,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(map),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(sampler),
        },
    ];
    if let Some((view, cube_sampler)) = cube {
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::TextureView(view),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::Sampler(cube_sampler),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("3D frame"),
        layout,
        entries: &entries,
    })
}
/// An environment-lit frame's layout (the frame's, then the cube and its
/// trilinear sampler) and that sampler.
pub(super) fn environment_layout(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::Sampler) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("3D frame, environment-lit"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("3D environment"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    (layout, sampler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, World, math::vec3};

    #[test]
    fn a_frame_shades_with_the_eight_point_lights_nearest_the_target() {
        let world = World::new();
        world.set_camera(Camera::looking_at([0., 0., 9.], [0., 0., 0.]));
        // Along x at these distances from the target; the second 1 is
        // added after the first.
        let lights = [
            (9., 1.),
            (5., 1.),
            (1., 1.),
            (7., 1.),
            (3., 1.),
            (2., 1.),
            (8., 1.),
        ];
        for (x, intensity) in lights
            .into_iter()
            .chain([(4., 1.), (6., 1.), (10., 1.), (1., 2.)])
        {
            world.add_light(Light::Point {
                position: vec3(x, 0., 0.),
                color: [1.; 3],
                intensity,
                range: 10.,
            });
        }
        let uniforms = frame_uniforms(&world.inner(), Mat4::IDENTITY, false);
        let shaded: Vec<(f32, f32)> = uniforms
            .points
            .iter()
            .map(|[position, color, _]| (position[0], color[3]))
            .collect();
        let nearest = [1., 1., 2., 3., 4., 5., 6., 7.];
        let intensities = [1., 2., 1., 1., 1., 1., 1., 1.];
        assert_eq!(
            shaded,
            nearest.into_iter().zip(intensities).collect::<Vec<_>>()
        );
        assert_eq!(uniforms.params[0], 8.);
    }
}
