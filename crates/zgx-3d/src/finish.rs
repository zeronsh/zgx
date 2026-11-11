//! A surface's finishes beyond metal and roughness: sheen (cloth, velvet),
//! anisotropy (brushed metal) and iridescence (soap films, oil): glTF's
//! `KHR_materials_sheen`, `KHR_materials_anisotropy` and
//! `KHR_materials_iridescence`, by their factors. (Their textures are not
//! read yet.) Set on a mesh by `World::set_sheen`, `set_anisotropy` and
//! `set_iridescence`.

/// A sheen (`World::set_sheen`): a soft lobe at grazing angles, as of the
/// fibres of cloth or velvet, of a colour (linear) and roughness, which
/// takes the energy it reflects out of what's under it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheen {
    pub(crate) color: [f32; 3],
    pub(crate) roughness: f32,
}
impl Sheen {
    /// A sheen of linear colour `color` (0 to 1 each), as rough as glTF's
    /// default (0).
    pub fn new(color: [f32; 3]) -> Self {
        Self {
            color: color.map(clamp01),
            roughness: 0.,
        }
    }
    /// Its roughness: 0 a narrow lobe at the very grazing edge, 1 a broad
    /// one.
    pub fn roughness(mut self, roughness: f32) -> Self {
        self.roughness = clamp01(roughness);
        self
    }
    /// Whether it shows at all.
    pub(crate) fn shows(&self) -> bool {
        self.color.iter().any(|c| *c > 0.)
    }
    pub(crate) fn words(&self) -> [f32; 4] {
        let [r, g, b] = self.color;
        [r, g, b, self.roughness]
    }
}

/// Anisotropy (`World::set_anisotropy`): highlights stretched along a
/// direction in the surface, as on brushed metal or hair. Its direction is
/// the mesh's texture's u axis turned by `rotation` (radians).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anisotropy {
    pub(crate) strength: f32,
    pub(crate) rotation: f32,
}
impl Anisotropy {
    /// As anisotropic as `strength` (0 to 1), along the texture's u axis.
    pub fn new(strength: f32) -> Self {
        Self {
            strength: clamp01(strength),
            rotation: 0.,
        }
    }
    /// Its direction turned from the texture's u axis by `radians`.
    pub fn rotation(mut self, radians: f32) -> Self {
        self.rotation = if radians.is_finite() { radians } else { 0. };
        self
    }
    pub(crate) fn words(&self) -> [f32; 4] {
        let (sin, cos) = self.rotation.sin_cos();
        [self.strength, cos, sin, 0.]
    }
}

/// Iridescence (`World::set_iridescence`): a thin film over the surface
/// whose reflection shifts in colour with the angle it's seen at, as soap
/// bubbles and oil on water do: how much, the film's index of refraction
/// and its thickness in nanometres.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Iridescence {
    pub(crate) factor: f32,
    pub(crate) ior: f32,
    pub(crate) thickness: f32,
}
impl Iridescence {
    /// As much film as `factor` (0 to 1), of glTF's default index (1.3) and
    /// thickness (400 nm).
    pub fn new(factor: f32) -> Self {
        Self {
            factor: clamp01(factor),
            ior: 1.3,
            thickness: 400.,
        }
    }
    /// The film's index of refraction (1 to 3).
    pub fn ior(mut self, ior: f32) -> Self {
        self.ior = if ior.is_finite() {
            ior.clamp(1., 3.)
        } else {
            1.3
        };
        self
    }
    /// The film's thickness in nanometres (0 to 2,000).
    pub fn thickness(mut self, nanometres: f32) -> Self {
        self.thickness = if nanometres.is_finite() {
            nanometres.clamp(0., 2000.)
        } else {
            400.
        };
        self
    }
    pub(crate) fn words(&self) -> [f32; 4] {
        [self.factor, self.ior, self.thickness, 0.]
    }
}

fn clamp01(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0., 1.) } else { 0. }
}
