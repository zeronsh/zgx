//! A world's painter: the renderer, and what runs round it. Before the first
//! frame drawn with each device it reads the device's [`Caps`]; with the
//! `compute` feature, it runs the app's compute passes marked before and
//! after the frame, in the frame's encoder on either side of the renderer's,
//! and moves the app's readbacks along. A world with no GPU data of its own
//! costs one flag and one test a frame.
use crate::{Caps, world::Inner};
use std::{cell::RefCell, rc::Weak};
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

pub(crate) struct Painter<V> {
    painter: V,
    world: Weak<RefCell<Inner>>,
    /// Whether this device's caps were read.
    read: bool,
    /// The app's GPU data on this device.
    #[cfg(feature = "compute")]
    data: crate::compute::Frames,
}
impl<V> Painter<V> {
    pub(crate) fn new(painter: V, world: Weak<RefCell<Inner>>) -> Self {
        Self {
            painter,
            world,
            read: false,
            #[cfg(feature = "compute")]
            data: crate::compute::Frames::default(),
        }
    }
    #[cold]
    #[inline(never)]
    fn read(&mut self, device: &wgpu::Device) {
        self.read = true;
        if let Some(world) = self.world.upgrade() {
            world.borrow().caps.set(Some(Caps::of(device)));
        }
    }
}
impl<V: Viewport> Viewport for Painter<V> {
    #[inline]
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        if !self.read {
            self.read(frame.device);
        }
        #[cfg(feature = "compute")]
        let data = self.data.before(&self.world, frame);
        self.painter.render(frame);
        #[cfg(feature = "compute")]
        if data {
            self.data.after(&self.world, frame);
        }
    }
    fn device_changed(&mut self) {
        self.read = false;
        #[cfg(feature = "compute")]
        self.data.device_changed();
        self.painter.device_changed();
    }
    fn rest(&mut self) {
        self.painter.rest();
    }
}
