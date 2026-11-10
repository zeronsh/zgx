//! What a world's frame goes through after its meshes are drawn: the post
//! chain (design-3d-aaa.md, 5.2 and 6.1), with the `post` feature.
use super::World;

/// The passes a world's frame goes through after its meshes
/// (`World::set_post`). Each is off by default; with any on, the meshes draw
/// into a scene target in linear light, unbounded, and a last pass exposes
/// it, maps its tones (`World::set_exposure`, `World::set_tone_mapping`)
/// and writes the view's target. Made with `..Default::default()` for what
/// isn't set, so passes added later leave it as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Post {
    /// Light spreading around what is bright, as a lens's glare does.
    pub bloom: Option<Bloom>,
}
impl Post {
    /// Whether any pass is on: if not, the meshes draw into the view's
    /// target as they do with no chain.
    pub(crate) fn any(&self) -> bool {
        self.bloom.is_some()
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

impl World {
    /// The passes its frame goes through after its meshes (none by
    /// default). With one on, meshes draw in linear light into a target of
    /// their own (8 bytes a pixel, with the passes' own targets beside it)
    /// and unlit meshes are exposed and tone mapped with the rest; the
    /// background still shows as given.
    pub fn set_post(&self, post: Post) {
        let post = Post {
            bloom: post.bloom.map(|bloom| Bloom {
                intensity: if bloom.intensity.is_finite() {
                    bloom.intensity.clamp(0., 1.)
                } else {
                    0.
                },
            }),
        };
        self.change_view(|inner| std::mem::replace(&mut inner.post, post) != post);
    }
}
