//! What a world's renderers are loading for it (`crate::assets`).
use super::{Settle, World};

impl World {
    /// Whether maps its last frame drew are still on their way to the GPU
    /// (decoded by threads of their own): until each arrives its meshes draw
    /// without it, in their material's own colours, and the world keeps
    /// drawing frames. False once a frame has drawn them all, and before
    /// any frame.
    pub fn loading(&self) -> bool {
        self.inner.borrow().settle.unsettled.get() & Settle::LOADING != 0
    }
}
