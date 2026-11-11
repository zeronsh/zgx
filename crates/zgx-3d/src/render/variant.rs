//! What sets a pipeline apart from the plain one.
/// What sets a pipeline apart from the plain one (`Variant::default()`),
/// as bits: a frame makes one for every mesh and compares and hashes them
/// a run, and one number costs those least.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct Variant(pub(super) u16);
impl Variant {
    /// It samples maps (or stand-ins).
    pub(super) const TEXTURED: u16 = 1 << 0;
    /// Its joints place its vertices.
    pub(super) const SKINNED: u16 = 1 << 1;
    /// Lit by an environment.
    pub(super) const LIT: u16 = 1 << 2;
    /// Its morph targets blend it.
    pub(super) const MORPHING: u16 = 1 << 3;
    /// Its back faces draw.
    pub(super) const TWO_SIDED: u16 = 1 << 4;
    /// Cut out by its base colour's alpha.
    pub(super) const MASKED: u16 = 1 << 5;
    /// Its maps placed (`World::set_map_placement`).
    pub(super) const PLACED: u16 = 1 << 6;
    /// Its geometry has vertex colours.
    pub(super) const COLORED: u16 = 1 << 7;
    /// Light passes through it (`World::set_transmission`).
    pub(super) const TRANSMITTING: u16 = 1 << 8;
    /// A clear coat over it (`World::set_clearcoat`).
    pub(super) const COATED: u16 = 1 << 9;
    /// It reflects as its specular says (`World::set_specular`).
    pub(super) const SPECULAR: u16 = 1 << 10;
    /// Its world shades point and spot lights through clusters (`lights`).
    pub(super) const CLUSTERED: u16 = 1 << 11;
    /// Its world's sun casts its shadow through cascades.
    pub(super) const CASCADED: u16 = 1 << 12;
    /// A custom material draws it (`World::set_shader`): which one, its
    /// binding says (`custom`).
    pub(super) const CUSTOM: u16 = 1 << 13;
    /// Its diffuse light from irradiance probes (`World::add_probes`).
    pub(super) const PROBES: u16 = 1 << 14;
    // `Extras::kinds`' bits, shifted, are these (see `variant_of`).
    pub(super) const _KINDS: () = assert!(
        Self::TRANSMITTING == crate::world::GLASS as u16 * Self::TRANSMITTING
            && Self::COATED == crate::world::COAT as u16 * Self::TRANSMITTING
            && Self::SPECULAR == crate::world::SPECULAR as u16 * Self::TRANSMITTING
            && Self::CUSTOM == crate::world::SHADER as u16 * Self::TRANSMITTING
    );
    /// Every bit there is (`shader`'s tests cover each, and fail on one
    /// left out of this).
    #[cfg(test)]
    pub(super) const ALL: u16 = (Self::PROBES << 1) - 1;
    /// It with `bit` on or off.
    pub(super) fn with(self, bit: u16, on: bool) -> Self {
        Self(if on { self.0 | bit } else { self.0 & !bit })
    }
    pub(super) fn textured(self) -> bool {
        self.0 & Self::TEXTURED != 0
    }
    pub(super) fn skinned(self) -> bool {
        self.0 & Self::SKINNED != 0
    }
    pub(super) fn lit(self) -> bool {
        self.0 & Self::LIT != 0
    }
    pub(super) fn morphing(self) -> bool {
        self.0 & Self::MORPHING != 0
    }
    pub(super) fn two_sided(self) -> bool {
        self.0 & Self::TWO_SIDED != 0
    }
    pub(super) fn masked(self) -> bool {
        self.0 & Self::MASKED != 0
    }
    pub(super) fn placed(self) -> bool {
        self.0 & Self::PLACED != 0
    }
    pub(super) fn colored(self) -> bool {
        self.0 & Self::COLORED != 0
    }
    pub(super) fn transmitting(self) -> bool {
        self.0 & Self::TRANSMITTING != 0
    }
    pub(super) fn coated(self) -> bool {
        self.0 & Self::COATED != 0
    }
    pub(super) fn specular(self) -> bool {
        self.0 & Self::SPECULAR != 0
    }
    pub(super) fn clustered(self) -> bool {
        self.0 & Self::CLUSTERED != 0
    }
    pub(super) fn cascaded(self) -> bool {
        self.0 & Self::CASCADED != 0
    }
    pub(super) fn custom(self) -> bool {
        self.0 & Self::CUSTOM != 0
    }
    pub(super) fn probed(self) -> bool {
        self.0 & Self::PROBES != 0
    }
}
/// Hashes a `Variant` by its own bits: they're few and its, and the
/// pipelines are looked up a run a frame.
pub(super) type ByVariant = std::hash::BuildHasherDefault<VariantHasher>;
#[derive(Default)]
pub(super) struct VariantHasher(u64);
impl std::hash::Hasher for VariantHasher {
    fn finish(&self) -> u64 {
        self.0.wrapping_mul(0x9e37_79b9_7f4a_7c15)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0 << 8 | u64::from(byte);
        }
    }
    fn write_u16(&mut self, bits: u16) {
        self.0 = u64::from(bits);
    }
    fn write_u8(&mut self, bits: u8) {
        self.0 = u64::from(bits);
    }
}
