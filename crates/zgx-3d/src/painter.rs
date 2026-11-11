//! A world's painter: the renderer, and what runs round it. Before the first
//! frame drawn with each device it reads the device's [`Caps`]; with the
//! `compute` feature, it runs the app's compute passes marked before and
//! after the frame, in the frame's encoder on either side of the renderer's,
//! and moves the app's readbacks along; with `particles`, it steps the
//! world's emitters before the frame (the renderer draws them with the
//! meshes). A world with
//! none of these costs one flag and one test a frame.
//!
//! What its frame holds unsettled is its own (`world::Settle`): a world drawn
//! by two painters draws on while either holds a reason, and a painter's go
//! as it rests or is dropped.
use crate::{Caps, world::Inner};
use std::{cell::RefCell, rc::Weak};
use zgx_gpu::{
    viewport::{Viewport, ViewportFrame},
    wgpu,
};

pub(crate) struct Painter<V> {
    painter: V,
    /// Its own among the world's painters (from 1: 0 is the world's).
    id: u64,
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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            painter,
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            world,
            read: false,
            #[cfg(feature = "compute")]
            data: crate::compute::Frames::default(),
            #[cfg(feature = "particles")]
            particles: crate::particles::Device::default(),
        }
    }
    /// Who sets the world's reasons to keep drawing: this painter (`on`)
    /// or the world itself.
    fn drawing(&self, on: bool) {
        if let Some(world) = self.world.upgrade()
            && let Ok(world) = world.try_borrow()
        {
            world.settle.drawing.set(if on { self.id } else { 0 });
        }
    }
    /// What it held unsettled let go.
    fn release(&self) {
        if let Some(world) = self.world.upgrade()
            && let Ok(world) = world.try_borrow()
        {
            world.release(self.id);
        }
    }
    #[cfg(test)]
    pub(crate) fn id(&self) -> u64 {
        self.id
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
        self.drawing(true);
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
            self.particles.after(&self.world);
        }
        #[cfg(feature = "compute")]
        if data {
            self.data.after(&self.world, frame);
        }
        self.drawing(false);
    }
    fn device_changed(&mut self) {
        self.read = false;
        #[cfg(feature = "compute")]
        self.data.device_changed(&self.world);
        #[cfg(feature = "particles")]
        self.particles.device_changed(&self.world);
        self.painter.device_changed();
    }
    fn rest(&mut self) {
        // It draws no more until shown: nothing waits on it.
        self.release();
        self.painter.rest();
    }
}
impl<V> Drop for Painter<V> {
    fn drop(&mut self) {
        // Gone: what it held goes with it, and the world's particles'
        // drawer keeps nothing of its renderer's.
        self.release();
        #[cfg(feature = "particles")]
        self.particles.dropped(&self.world);
    }
}
