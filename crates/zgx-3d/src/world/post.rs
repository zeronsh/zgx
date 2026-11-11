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
    /// Exposure that follows the scene's light, as an eye or camera
    /// adapts, instead of `World::set_exposure`'s stops alone.
    pub auto_exposure: Option<AutoExposure>,
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
            || self.auto_exposure.is_some()
    }
}

impl super::Settle {
    /// Automatic exposure is still adapting (`Post::auto_exposure`).
    pub(crate) const EXPOSURE: u32 = super::Reason::Exposure as u32;
}

/// Exposure following the scene's light (`Post::auto_exposure`): a
/// histogram of the scene's brightness each frame, the mean of its middle
/// (between `low` and `high`, so a few very bright or dark pixels don't
/// swing it) brought to middle grey, eased toward at `up` and `down` stops
/// a second. `World::set_exposure`'s stops still apply, as compensation
/// on top. Where the device runs no compute shaders (`World::caps`), the
/// stops alone apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposure {
    /// The dimmest scene it adapts to, as log2 of its mean luminance in the
    /// scene's units (a scene whose mean is 1 is 0, and is exposed by
    /// log2 0.18, to middle grey); dimmer, it's exposed as that one. −8 by
    /// default.
    pub min: f32,
    /// The brightest, likewise. 8 by default.
    pub max: f32,
    /// The share of the histogram's pixels, from the darkest, left out
    /// below the mean: 0.5 by default (the mean of the brighter half).
    pub low: f32,
    /// The share, from the darkest, above which pixels are left out too:
    /// 0.9 by default (the brightest tenth left out).
    pub high: f32,
    /// How fast it adapts to a brighter scene, in stops a second: 3 by
    /// default (eyes adapt to light faster than to the dark).
    pub up: f32,
    /// How fast it adapts to a darker scene: 1 by default.
    pub down: f32,
}
impl Default for AutoExposure {
    fn default() -> Self {
        Self {
            min: -8.,
            max: 8.,
            low: 0.5,
            high: 0.9,
            up: 3.,
            down: 1.,
        }
    }
}
impl AutoExposure {
    /// The same, every number finite and in order.
    fn sanitized(self) -> Self {
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        let d = Self::default();
        let min = finite(self.min, d.min).clamp(-30., 30.);
        let low = finite(self.low, d.low).clamp(0., 0.99);
        Self {
            min,
            max: finite(self.max, d.max).clamp(min, 30.),
            low,
            high: finite(self.high, d.high).clamp(low + 0.01, 1.),
            up: finite(self.up, d.up).clamp(0.01, 100.),
            down: finite(self.down, d.down).clamp(0.01, 100.),
        }
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
            auto_exposure: post.auto_exposure.map(AutoExposure::sanitized),
        };
        let adapts = post.auto_exposure.is_some();
        self.change_view(|inner| {
            let changed = inner.post != post;
            inner.post = post;
            changed
        });
        // Unsettled until its exposure is where it's going (the renderer
        // reads that back).
        self.inner
            .borrow()
            .unsettle(super::Settle::EXPOSURE, adapts);
    }
    /// The exposure automatic exposure has brought it to
    /// (`Post::auto_exposure`), in stops as `set_exposure` takes them: what
    /// the scene's light is exposed by before tone mapping, `set_exposure`'s
    /// stops included (log2 0.18, less log2 of the scene's mean luminance
    /// adapted to, plus them). As a painter last read it back from the
    /// device, a frame or two after drawing it; `None` before then, with
    /// automatic exposure off, or where the device runs no compute shaders
    /// (`World::caps`; `set_exposure`'s stops alone apply there).
    pub fn adapted_exposure(&self) -> Option<f32> {
        let inner = self.inner.borrow();
        inner.post.auto_exposure?;
        let adapted = inner.adapted.get()?;
        Some(0.18f32.log2() - adapted + inner.exposure)
    }
}
