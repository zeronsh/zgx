//! A world's painter: the renderer, and what runs round it. Before the first
//! frame drawn with each device it reads the device's [`Caps`]; with the
//! `compute` feature, it runs the app's compute passes marked before and
//! after the frame, in the frame's encoder on either side of the renderer's,
//! and moves the app's readbacks along; with `particles`, it steps the
//! world's emitters before the frame and draws them after. A world with
//! none of these costs one flag and one test a frame.
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
    /// The world's particles on this device.
    #[cfg(feature = "particles")]
    particles: crate::particles::Device,
}
impl<V> Painter<V> {
    pub(crate) fn new(painter: V, world: Weak<RefCell<Inner>>) -> Self {
        Self {
            painter,
            world,
            read: false,
            #[cfg(feature = "compute")]
            data: crate::compute::Frames::default(),
            #[cfg(feature = "particles")]
            particles: crate::particles::Device::default(),
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
        #[cfg(feature = "particles")]
        let particles = self.particles.before(&self.world, frame);
        self.painter.render(frame);
        // The world's sound follows what this frame showed.
        #[cfg(feature = "audio")]
        if let Some(world) = self.world.upgrade() {
            crate::world::audio::frame(&world, frame.time);
        }
        #[cfg(feature = "particles")]
        if particles {
            self.particles.after(&self.world, frame);
        }
        #[cfg(feature = "compute")]
        if data {
            self.data.after(&self.world, frame);
        }
    }
    fn device_changed(&mut self) {
        self.read = false;
        #[cfg(feature = "compute")]
        self.data.device_changed();
        #[cfg(feature = "particles")]
        self.particles.device_changed();
        self.painter.device_changed();
    }
    fn rest(&mut self) {
        self.painter.rest();
    }
}
