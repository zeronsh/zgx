//! The lighting milestone's settings on a world (design-3d-aaa.md, 6.3):
//! clustered lights. Behind the `lighting` feature.
use super::{Inner, World};

/// A world's lighting settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Lighting {
    /// Point and spot lights shaded through clusters whatever their count
    /// (`World::set_clustered_lights`).
    pub(crate) clustered: bool,
}

impl World {
    /// Shade point and spot lights through clusters (on) or, with eight or
    /// fewer, the uniform path's eight (off, the default).
    ///
    /// A world with more point lights than [`POINT_LIGHTS`](crate::POINT_LIGHTS)
    /// shades through clusters either way: its view is cut into 16 by 9
    /// tiles and 24 slices of depth, each light listed in the clusters it
    /// reaches, and each pixel shades with its own cluster's lights alone, so
    /// hundreds of lights cost about what the few reaching each pixel do. The
    /// lists are made on the CPU, again only when a light, the camera or the
    /// view's size changes. A device without storage buffers in its shaders
    /// (some GL) shades the eight nearest the camera's target, as without
    /// the `lighting` feature.
    pub fn set_clustered_lights(&self, on: bool) {
        self.change_view(|inner: &mut Inner| {
            std::mem::replace(&mut inner.lighting.clustered, on) != on
        });
    }
    /// Whether point and spot lights shade through clusters whatever their
    /// count (`set_clustered_lights`).
    pub fn clustered_lights(&self) -> bool {
        self.inner.borrow().lighting.clustered
    }
}
