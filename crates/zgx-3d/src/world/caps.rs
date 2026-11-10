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

impl World {
    /// Its state, to change: for the modules of features kept apart from
    /// world.rs (compute, particles).
    #[cfg_attr(not(feature = "compute"), allow(dead_code))]
    pub(crate) fn inner_mut(&self) -> std::cell::RefMut<'_, super::Inner> {
        self.inner.borrow_mut()
    }
}
