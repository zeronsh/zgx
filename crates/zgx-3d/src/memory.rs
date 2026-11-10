//! `World::memory`: what a world holds on the device, by kind
//! (design-3d-aaa.md, 4.3).

/// Bytes a world holds on the device, by kind ([`World::memory`]).
///
/// [`World::memory`]: crate::World::memory
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    /// The frame's temporary targets (colour, depth, what is behind glass):
    /// none while its window rests.
    pub targets: u64,
    /// The app's buffers (`World::add_buffer`), live handles only.
    pub app_buffers: u64,
    /// The app's textures (`World::add_texture`), live handles only.
    pub app_textures: u64,
    /// Particle emitters' pools (`World::add_emitter`).
    pub particles: u64,
}
impl Memory {
    /// The app's buffers and textures: what counts against the world's cap
    /// (`World::set_memory_cap`).
    pub fn app(&self) -> u64 {
        self.app_buffers + self.app_textures
    }
}

impl crate::World {
    /// Bytes it holds on the device, by kind. Handles the app dropped are let
    /// go here, as by the next frame.
    pub fn memory(&self) -> Memory {
        #[allow(unused_mut)]
        let mut memory = Memory {
            targets: self.temporary_bytes(),
            ..Memory::default()
        };
        #[cfg(feature = "compute")]
        {
            let mut inner = self.inner_mut();
            if let Some(data) = inner.data.as_deref_mut() {
                data.sweep();
                memory.app_buffers = data.buffer_bytes;
                memory.app_textures = data.texture_bytes;
            }
        }
        #[cfg(feature = "particles")]
        {
            let on_gpu = self.caps().is_none_or(|caps| caps.compute);
            let inner = self.inner_mut();
            if let Some(particles) = inner.particles.as_deref() {
                memory.particles = particles.bytes(on_gpu);
            }
        }
        memory
    }
}
