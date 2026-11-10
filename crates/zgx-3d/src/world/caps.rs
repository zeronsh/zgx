//! `World::caps`: what its view's device can do (`crate::caps`).
use super::World;
use crate::Caps;

impl World {
    /// What the device its view draws with can do: `None` until a frame has
    /// been drawn (the device is the window's, known only then), and read
    /// again when the view moves to another device.
    pub fn caps(&self) -> Option<Caps> {
        self.inner.borrow().caps.get()
    }
}
