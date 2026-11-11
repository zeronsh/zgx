//! A mesh's finishes (`World::set_sheen`, `set_anisotropy`,
//! `set_iridescence`), kept beside its coat and specular.
use super::{Layers, ObjectId, World};
use crate::{Anisotropy, Iridescence, Sheen};

impl World {
    /// Give mesh `id` a sheen (`None`: none), as glTF's
    /// `KHR_materials_sheen` does.
    pub fn set_sheen(&self, id: ObjectId, sheen: Option<Sheen>) {
        self.set_finish(id, sheen.filter(Sheen::shows), |l| &mut l.sheen);
    }
    /// Mesh `id`'s sheen, if it has one.
    pub fn sheen(&self, id: ObjectId) -> Option<Sheen> {
        self.finish_of(id, |l| l.sheen)
    }
    /// Stretch mesh `id`'s highlights (`None`: as round as its roughness),
    /// as glTF's `KHR_materials_anisotropy` does.
    pub fn set_anisotropy(&self, id: ObjectId, anisotropy: Option<Anisotropy>) {
        let anisotropy = anisotropy.filter(|a| a.strength > 0.);
        self.set_finish(id, anisotropy, |l| &mut l.anisotropy);
    }
    /// Mesh `id`'s anisotropy, if it has one.
    pub fn anisotropy(&self, id: ObjectId) -> Option<Anisotropy> {
        self.finish_of(id, |l| l.anisotropy)
    }
    /// Give mesh `id` a thin film's iridescence (`None`: none), as glTF's
    /// `KHR_materials_iridescence` does.
    pub fn set_iridescence(&self, id: ObjectId, iridescence: Option<Iridescence>) {
        let iridescence = iridescence.filter(|i| i.factor > 0.);
        self.set_finish(id, iridescence, |l| &mut l.iridescence);
    }
    /// Mesh `id`'s iridescence, if it has one.
    pub fn iridescence(&self, id: ObjectId) -> Option<Iridescence> {
        self.finish_of(id, |l| l.iridescence)
    }
    /// Mesh `id`'s layer `field` set to `value`: a frame if it changed.
    fn set_finish<T: PartialEq + Copy>(
        &self,
        id: ObjectId,
        value: Option<T>,
        field: impl FnOnce(&mut Layers) -> &mut Option<T>,
    ) {
        self.change(|inner| {
            if inner.live(id.index, id.generation).is_none() {
                return false;
            }
            let extras = inner.extras_mut(id.index, id.generation);
            let before = extras.change_layers(|l| std::mem::replace(field(l), value));
            inner.tidy_extras(id.index);
            before != value
        });
    }
    fn finish_of<T: Copy>(
        &self,
        id: ObjectId,
        field: impl FnOnce(&Layers) -> Option<T>,
    ) -> Option<T> {
        let inner = self.inner.borrow();
        field(
            inner
                .extras(id.index as usize, id.generation)?
                .layers
                .as_deref()?,
        )
    }
}
