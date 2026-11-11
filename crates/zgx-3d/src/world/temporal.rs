//! Temporal antialiasing (design-3d-aaa.md, 6.2): each frame drawn a
//! fraction of a pixel aside, and blended with the frames before it moved
//! to where they are now.
use super::World;

/// How a world smooths its edges (`World::set_antialiasing`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Antialiasing {
    /// Four samples a pixel (multisampling): edges of meshes smooth, at once.
    #[default]
    Msaa,
    /// One sample a pixel, a frame drawn a fraction of a pixel aside each
    /// time and blended with those before (temporal): every edge and
    /// highlight smooth, inside meshes too, after the 8 frames it keeps
    /// drawing once a change stops. Turns the post chain on. **Limit until
    /// the depth prepass:** only the camera's motion is followed (from the
    /// depth); a mesh moving, skinned or morphing is held to this frame's
    /// neighbourhood alone, so a fast one may leave a faint trail.
    Taa,
    /// One sample a pixel, hard edges.
    None,
}

impl super::Settle {
    /// Temporal antialiasing has fewer than its 8 frames since a change.
    pub(crate) const TAA: u32 = 1 << 10;
}

impl World {
    /// How its edges are smoothed (`Antialiasing::Msaa` by default).
    /// Changing it makes its pipelines and targets again with the next frame
    /// and drops temporal history.
    pub fn set_antialiasing(&self, antialiasing: Antialiasing) {
        let (samples, taa) = match antialiasing {
            Antialiasing::Msaa => (4, false),
            Antialiasing::Taa => (1, true),
            Antialiasing::None => (1, false),
        };
        self.change_view(|inner| {
            let changed = inner.samples != samples || inner.taa != taa;
            inner.samples = samples;
            inner.taa = taa;
            changed
        });
        if !taa {
            self.inner.borrow().unsettle(super::Settle::TAA, false);
        }
    }
    /// How its edges are smoothed.
    pub fn antialiasing(&self) -> Antialiasing {
        let inner = self.inner.borrow();
        match (inner.taa, inner.samples) {
            (true, _) => Antialiasing::Taa,
            (false, 1) => Antialiasing::None,
            _ => Antialiasing::Msaa,
        }
    }
    /// A cut: what was drawn before isn't blended into the next frame
    /// (temporal antialiasing's history goes), as where the camera jumps.
    /// Frame gaps over a quarter of a second cut by themselves.
    pub fn cut(&self) {
        self.change_view(|inner| {
            inner.cuts += 1;
            true
        });
    }
}
