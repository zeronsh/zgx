//! The sun's cascades (design-3d-aaa.md, 6.3): the camera's view cut into
//! up to four slices by depth, each shadowed by a map of its own, the
//! nearer the sharper. Each cascade holds its slice's bounding sphere, so
//! turning the camera doesn't change its size, and moves only by whole
//! texels of its map, so a moving camera doesn't make shadow edges swim.
use crate::math::{Mat4, Vec3};
use zgx_gpu::wgpu;

/// The most cascades a sun casts.
pub(crate) const MAX_CASCADES: usize = 4;

/// Where each cascade starts and the last ends, in depth in front of the
/// camera: between `near` and `far`, `lambda` of the way from evenly
/// spaced to logarithmically (the practical split scheme: near cascades
/// small, far ones not too big).
pub(crate) fn splits(near: f32, far: f32, count: usize, lambda: f32) -> [f32; MAX_CASCADES + 1] {
    let count = count.clamp(1, MAX_CASCADES);
    let near = near.max(1e-3);
    let far = far.max(near * 1.01);
    let mut out = [far; MAX_CASCADES + 1];
    for (i, split) in out.iter_mut().enumerate().take(count + 1) {
        let t = i as f32 / count as f32;
        let logarithmic = near * (far / near).powf(t);
        let even = near + (far - near) * t;
        *split = lambda * logarithmic + (1. - lambda) * even;
    }
    out[0] = near;
    out
}

/// What the camera sees: where it is, which way it looks, and its view's
/// size at a distance of 1 (half height and half width; orthographic: its
/// half height and width whatever the distance).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Eye {
    pub(crate) position: Vec3,
    pub(crate) forward: Vec3,
    pub(crate) up: Vec3,
    pub(crate) right: Vec3,
    pub(crate) half: [f32; 2],
    pub(crate) orthographic: bool,
}
impl Eye {
    pub(crate) fn new(camera: &crate::Camera, aspect: f32) -> Self {
        let forward = (camera.target - camera.position).normalized();
        let mut right = forward.cross(Vec3::Y).normalized();
        if right == Vec3::ZERO {
            right = forward.cross(crate::vec3(0., 0., -1.)).normalized();
        }
        let up = right.cross(forward);
        let aspect = if aspect.is_finite() && aspect > 0. {
            aspect
        } else {
            1.
        };
        let h = if camera.orthographic {
            camera.fov * 0.5
        } else {
            (camera.fov.clamp(1e-3, 3.1) * 0.5).tan()
        };
        Self {
            position: camera.position,
            forward,
            up,
            right,
            half: [h * aspect, h],
            orthographic: camera.orthographic,
        }
    }
    /// The corners of what it sees between depths `near` and `far`.
    fn corners(&self, near: f32, far: f32) -> [Vec3; 8] {
        let mut out = [Vec3::ZERO; 8];
        for (i, corner) in out.iter_mut().enumerate() {
            let depth = if i & 4 == 0 { near } else { far };
            let scale = if self.orthographic { 1. } else { depth };
            let x = if i & 1 == 0 { -1. } else { 1. } * self.half[0] * scale;
            let y = if i & 2 == 0 { -1. } else { 1. } * self.half[1] * scale;
            *corner = self.position + self.forward * depth + self.right * x + self.up * y;
        }
        out
    }
}

/// The sun's frame, going `direction`: a turn alone (no place), the same
/// whatever the camera does, so cascades fit in it move only as the
/// camera does.
pub(crate) fn sun_frame(direction: Vec3) -> Mat4 {
    let d = direction.normalized();
    // Up: whichever axis is least along the sun.
    let up = if d.y.abs() < 0.99 {
        Vec3::Y
    } else {
        crate::vec3(1., 0., 0.)
    };
    Mat4::look_at(Vec3::ZERO, d, up)
}

/// A cascade: its view onto its map, and how wide a texel of its map is in
/// the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fit {
    pub(crate) matrix: Mat4,
    pub(crate) texel: f32,
}

/// The cascade holding what `eye` sees between depths `near` and `far`, as
/// the sun (its frame `frame`, `sun_frame`) sees it onto a map `side`
/// texels square, deep enough for everything in `scene` (the casters' box)
/// between the sun and it.
pub(crate) fn fit(
    eye: &Eye,
    (near, far): (f32, f32),
    frame: &Mat4,
    scene: (Vec3, Vec3),
    side: u32,
) -> Fit {
    let corners = eye.corners(near, far);
    // The slice's bounding sphere: its corners' middle, and the farthest of
    // them, rounded up to a 16th so a turning camera doesn't change it.
    let centre = corners.iter().fold(Vec3::ZERO, |sum, c| sum + *c) * (1. / 8.);
    let radius = corners
        .iter()
        .map(|c| (*c - centre).length())
        .fold(0., f32::max);
    let radius = (radius * 16.).ceil() / 16.;
    // Its middle in the sun's frame, moved to a whole texel.
    let texel = 2. * radius / side as f32;
    let middle = frame.project(centre);
    let snap = |v: f32| (v / texel).floor() * texel;
    let (x, y) = (snap(middle.x), snap(middle.y));
    // Depth: everything in the scene along the sun (view space looks down
    // -z: the nearest the sun is the greatest z), with the slice's own.
    let (low, high) = super::box_through(frame, scene);
    let (n, f) = if low.is_finite() && high.is_finite() {
        (
            -high.z.max(middle.z + radius),
            -low.z.min(middle.z - radius),
        )
    } else {
        (-(middle.z + radius), -(middle.z - radius))
    };
    let margin = (f - n) * 1e-3 + texel;
    let (n, f) = (n - margin, f + margin);
    let (l, r, b, t) = (x - radius, x + radius, y - radius, y + radius);
    let projection = Mat4 {
        columns: [
            [2. / (r - l), 0., 0., 0.],
            [0., 2. / (t - b), 0., 0.],
            [0., 0., 1. / (n - f), 0.],
            [-(r + l) / (r - l), -(t + b) / (t - b), n / (n - f), 1.],
        ],
    };
    Fit {
        matrix: projection * *frame,
        texel,
    }
}

/// A cascade map's depth: 16 bits, which a cascade's depth (fit to the
/// scene along the sun) holds finely enough, at half today's map's bytes.
pub(crate) const CASCADE_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// The main pass's cascades (the shader's `Cascades`): each one's view onto
/// its map, how far a lookup sits off a surface (as `shadow_texel`'s
/// third and fourth words), and how many and a texel's width in a map.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct CascadeUniforms {
    pub(crate) matrices: [[[f32; 4]; 4]; MAX_CASCADES],
    pub(crate) offsets: [[f32; 4]; MAX_CASCADES],
    pub(crate) params: [f32; 4],
}

/// What a cascade's map holds: its view, the casters it held (by instance,
/// `None`: all), how many instances there were, and the sun's direction.
pub(crate) struct Held {
    pub(crate) fit: Fit,
    pub(crate) mask: Option<Vec<bool>>,
    pub(crate) instances: usize,
    pub(crate) sun: Vec3,
}

/// What a renderer keeps for the sun's cascades, made when they turn on.
pub(crate) struct Sun {
    pub(crate) settings: crate::Cascades,
    /// The maps, a layer each; each layer's view, and all as an array.
    _texture: wgpu::Texture,
    pub(crate) layers: Vec<wgpu::TextureView>,
    pub(crate) array: wgpu::TextureView,
    /// The main pass's `CascadeUniforms`.
    pub(crate) uniforms: wgpu::Buffer,
    /// Each cascade's own frame for its caster pass (the frame's uniforms
    /// with its view as `shadow`), bound as today's map's pass binds its.
    pub(crate) casters: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    /// What each map holds, and whether this frame draws it again (with
    /// its casters, `None`: all).
    pub(crate) held: [Option<Held>; MAX_CASCADES],
    pub(crate) redraw: [bool; MAX_CASCADES],
    pub(crate) masks: [Vec<bool>; MAX_CASCADES],
    /// The spans of each run each cascade draws (`cull::shown`).
    pub(crate) spans: [Vec<(u32, std::ops::Range<u32>)>; MAX_CASCADES],
    /// Frames each far cascade has waited to follow the camera.
    pub(crate) waited: [u32; MAX_CASCADES],
    /// The casters' box along the sun, as of the last frame that drew the
    /// meshes again.
    pub(crate) scene: Option<(Vec3, Vec3)>,
    /// Their pipelines, by `shadow_key`.
    pub(crate) pipelines: std::collections::HashMap<u8, wgpu::RenderPipeline>,
    /// Maps drawn, for tests.
    pub(crate) draws: u64,
}

impl Sun {
    /// The maps and buffers for `settings`.
    #[cold]
    #[inline(never)]
    pub(crate) fn new(
        device: &wgpu::Device,
        shadow_layout: &wgpu::BindGroupLayout,
        settings: crate::Cascades,
    ) -> Self {
        let count = settings.count.clamp(1, MAX_CASCADES as u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3D sun cascades"),
            size: wgpu::Extent3d {
                width: settings.size,
                height: settings.size,
                depth_or_array_layers: count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CASCADE_DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let layers = (0..count)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("3D sun cascade"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let array = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("3D sun cascades"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("3D sun cascades"),
            size: std::mem::size_of::<CascadeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_size = std::mem::size_of::<super::super::frame::FrameUniforms>() as u64;
        let casters = (0..count)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("3D sun cascade's frame"),
                    size: frame_size,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3D sun cascade's frame"),
                    layout: shadow_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, bind)
            })
            .collect();
        Self {
            settings,
            _texture: texture,
            layers,
            array,
            uniforms,
            casters,
            held: Default::default(),
            redraw: [false; MAX_CASCADES],
            masks: Default::default(),
            spans: Default::default(),
            waited: [0; MAX_CASCADES],
            scene: None,
            pipelines: Default::default(),
            draws: 0,
        }
    }
    pub(crate) fn count(&self) -> usize {
        self.layers.len()
    }
}

/// What changed this frame, for `Sun::decide`.
pub(crate) struct Frame<'a> {
    /// The fits wanted now, one a cascade.
    pub(crate) fits: &'a [Fit],
    pub(crate) sun: Vec3,
    /// The instances written this frame (none where the meshes were kept),
    /// and how many there are.
    pub(crate) changed: &'a [std::ops::Range<usize>],
    pub(crate) instances: usize,
    /// Each cascade's casters now, if the meshes' boxes are current.
    pub(crate) masked: bool,
}

impl Sun {
    /// Which cascades this frame draws again (`redraw`), and what they then
    /// hold: one whose casters changed, or the sun, at once; the nearest as
    /// soon as the camera moves it; of the farther ones the camera moved,
    /// the one waiting longest. Whether any is left waiting (the world then
    /// draws again to catch up).
    pub(crate) fn decide(&mut self, frame: &Frame) -> bool {
        let mut waiting = None::<(u32, usize)>;
        for c in 0..self.count() {
            let fit = frame.fits[c];
            let mask = frame.masked.then(|| &self.masks[c][..]);
            let now = match &self.held[c] {
                None => true,
                Some(held) => {
                    let sun = held.sun != frame.sun || held.instances != frame.instances;
                    // A caster written this frame that it held or now holds.
                    let touched = |m: Option<&[bool]>| match m {
                        None => !frame.changed.is_empty(),
                        Some(m) => frame
                            .changed
                            .iter()
                            .any(|r| m[r.start.min(m.len())..r.end.min(m.len())].contains(&true)),
                    };
                    sun || touched(held.mask.as_deref()) || touched(mask)
                }
            };
            let moved = self.held[c].as_ref().is_some_and(|held| held.fit != fit);
            self.redraw[c] = now || (moved && c == 0);
            if !self.redraw[c] && moved {
                self.waited[c] += 1;
                if waiting.is_none_or(|(w, _)| self.waited[c] > w) {
                    waiting = Some((self.waited[c], c));
                }
            }
        }
        if let Some((_, c)) = waiting {
            self.redraw[c] = true;
        }
        let mut left = false;
        for c in 0..self.count() {
            if self.redraw[c] {
                self.waited[c] = 0;
                self.held[c] = Some(Held {
                    fit: frame.fits[c],
                    mask: frame.masked.then(|| self.masks[c].clone()),
                    instances: frame.instances,
                    sun: frame.sun,
                });
            } else {
                left |= self.waited[c] > 0;
            }
        }
        left
    }
    /// The main pass's uniforms: each map's view as it holds it.
    pub(crate) fn uniforms(&self) -> CascadeUniforms {
        let mut out = CascadeUniforms {
            matrices: [Mat4::IDENTITY.columns; MAX_CASCADES],
            offsets: [[0.; 4]; MAX_CASCADES],
            params: [self.count() as f32, 1. / self.settings.size as f32, 0., 0.],
        };
        for (c, held) in self.held.iter().enumerate().take(self.count()) {
            if let Some(held) = held {
                out.matrices[c] = held.fit.matrix.columns;
                // As today's map's: 1.5 texels facing the sun, up to 6 more
                // turned from it (`sunlit`).
                out.offsets[c] = [
                    held.fit.texel,
                    0.,
                    held.fit.texel * 7.5,
                    held.fit.texel * 6.,
                ];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, vec3};

    #[test]
    fn splits_run_near_to_far_closer_together_near() {
        let s = splits(0.1, 100., 4, 0.75);
        assert_eq!((s[0], s[4]), (0.1, 100.));
        for i in 0..4 {
            assert!(s[i] < s[i + 1]);
        }
        assert!(s[1] - s[0] < s[2] - s[1] && s[2] - s[1] < s[3] - s[2]);
        // Fewer: the rest at the far end.
        let two = splits(0.1, 50., 2, 0.5);
        assert_eq!((two[2], two[3], two[4]), (50., 50., 50.));
    }

    fn scene() -> (Vec3, Vec3) {
        (vec3(-50., 0., -50.), vec3(50., 20., 50.))
    }

    #[test]
    fn a_cascade_holds_its_slice_of_the_view() {
        let sun = vec3(-0.4, -1., -0.3);
        let frame = sun_frame(sun);
        for orthographic in [false, true] {
            let camera = Camera {
                orthographic,
                fov: if orthographic { 20. } else { 0.9 },
                ..Camera::looking_at([3., 4., 12.], [0., 0., 0.])
            };
            let eye = Eye::new(&camera, 16. / 9.);
            let s = splits(camera.near, 60., 4, 0.75);
            for i in 0..4 {
                let fit = fit(&eye, (s[i], s[i + 1]), &frame, scene(), 1024);
                for c in eye.corners(s[i], s[i + 1]) {
                    let p = fit.matrix.project(c);
                    assert!(
                        p.x.abs() <= 1. && p.y.abs() <= 1. && (0. ..=1.).contains(&p.z),
                        "{p:?}"
                    );
                }
                // And the scene's top above it, towards the sun.
                let p = fit
                    .matrix
                    .project(eye.corners(s[i], s[i + 1])[0] - sun * 5.);
                assert!(p.z >= 0., "a caster above it: {p:?}");
            }
        }
    }

    #[test]
    fn a_cascade_moves_by_whole_texels_and_keeps_its_size_as_the_camera_moves() {
        let frame = sun_frame(vec3(-0.4, -1., -0.3));
        let side = 1024;
        let mut last: Option<Fit> = None;
        // A slow pan and turn: a centimetre and a tenth of a degree a step.
        for step in 0..200 {
            let t = step as f32;
            let at = vec3(3. + t * 0.01, 4., 12.);
            let camera = Camera::looking_at(at, vec3(t * 0.002, 0., 0.));
            let fit = fit(&Eye::new(&camera, 1.5), (0.5, 8.), &frame, scene(), side);
            if let Some(last) = last {
                assert_eq!(fit.texel, last.texel, "its size held");
                // The world's origin on the map, before and after: moved by
                // whole texels (of 2 / side in clip space).
                let (a, b) = (
                    last.matrix.project(Vec3::ZERO),
                    fit.matrix.project(Vec3::ZERO),
                );
                for d in [b.x - a.x, b.y - a.y] {
                    let texels = d * side as f32 / 2.;
                    assert!((texels - texels.round()).abs() < 1e-2, "{texels}");
                }
            }
            last = Some(fit);
        }
    }
}
