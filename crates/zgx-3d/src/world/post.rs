//! What a world's frame goes through after its meshes are drawn: the post
//! chain (design-3d-aaa.md, 5.2 and 6.1), with the `post` feature.
use super::{Grade, World};

/// The passes a world's frame goes through after its meshes
/// (`World::set_post`). Each is off by default; with any on, the meshes draw
/// into a scene target in linear light, unbounded, and a last pass exposes
/// it, maps its tones (`World::set_exposure`, `World::set_tone_mapping`)
/// and writes the view's target. Made with `..Default::default()` for what
/// isn't set, so passes added later leave it as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Post {
    /// Light spreading around what is bright, as a lens's glare does.
    pub bloom: Option<Bloom>,
    /// The frame darker toward its corners, as a lens's is.
    pub vignette: Option<Vignette>,
    /// Film's grain: noise in the middle tones, a function of the frame's
    /// index (a recorded frame repeats).
    pub grain: Option<Grain>,
    /// Colours parted toward the frame's edges, red outward and blue
    /// inward, as a simple lens refracts them.
    pub chromatic_aberration: Option<ChromaticAberration>,
    /// Colour grading: white balance, lift, gamma, gain, saturation and a
    /// look-up table.
    pub grade: Option<Grade>,
}
impl Post {
    /// Whether any pass is on: if not, the meshes draw into the view's
    /// target as they do with no chain.
    pub(crate) fn any(&self) -> bool {
        self.bloom.is_some()
            || self.vignette.is_some()
            || self.grain.is_some()
            || self.chromatic_aberration.is_some()
            || self.grade.is_some()
    }
}

/// An intensity from 0 to 1, 0 where it isn't a number.
fn unit(intensity: f32) -> f32 {
    if intensity.is_finite() {
        intensity.clamp(0., 1.)
    } else {
        0.
    }
}

/// Light spreading around what is bright: the scene blurred wider and
/// wider (each step half the size of the last), the steps averaged, and
/// mixed into it by `intensity`. A mix, not a sum: no light is added, so
/// only what is far brighter than its surroundings glows visibly, with no
/// threshold to tune.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    /// How much of the blurred scene is mixed in: 0 none, 1 only it. 0.04
    /// by default.
    pub intensity: f32,
}
impl Default for Bloom {
    fn default() -> Self {
        Self { intensity: 0.04 }
    }
}

/// The frame darker toward its corners: light falls as the fourth power
/// of the cosine of its angle off the lens's axis, as through a real lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vignette {
    /// 0 none; at 1 the corners have a quarter of the light. 0.4 by
    /// default.
    pub intensity: f32,
}
impl Default for Vignette {
    fn default() -> Self {
        Self { intensity: 0.4 }
    }
}

/// Film's grain, after tone mapping, most in the middle tones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grain {
    /// How strong: 0 none, 1 coarse. 0.05 by default.
    pub intensity: f32,
}
impl Default for Grain {
    fn default() -> Self {
        Self { intensity: 0.05 }
    }
}

/// Colours parted toward the frame's edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromaticAberration {
    /// How far: at 1, red and blue each a hundredth of the frame's width
    /// apart from green at its edges. 0.3 by default.
    pub intensity: f32,
}
impl Default for ChromaticAberration {
    fn default() -> Self {
        Self { intensity: 0.3 }
    }
}

impl World {
    /// The passes its frame goes through after its meshes (none by
    /// default). With one on, meshes draw in linear light into a target of
    /// their own (8 bytes a pixel, with the passes' own targets beside it)
    /// and unlit meshes are exposed and tone mapped with the rest; the
    /// background still shows as given.
    pub fn set_post(&self, post: Post) {
        let post = Post {
            bloom: post.bloom.map(|b| Bloom {
                intensity: unit(b.intensity),
            }),
            vignette: post.vignette.map(|v| Vignette {
                intensity: unit(v.intensity),
            }),
            grain: post.grain.map(|g| Grain {
                intensity: unit(g.intensity),
            }),
            chromatic_aberration: post.chromatic_aberration.map(|c| ChromaticAberration {
                intensity: unit(c.intensity),
            }),
            grade: post.grade.as_ref().map(Grade::sanitized),
        };
        self.change_view(|inner| {
            let changed = inner.post != post;
            inner.post = post;
            changed
        });
    }
}
