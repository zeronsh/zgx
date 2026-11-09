//! objc2-metal helpers (macOS, `metal`): the system's Metal devices by
//! `registryID`, and an IOSurface as a Metal texture on a device. No wgpu:
//! a client on wgpu 30 (whose wgpu-hal is on objc2-metal 0.3 too) makes its
//! textures with `texture` and wraps them with its own wgpu-hal's
//! `texture_from_raw`, as `wgpu29` does.
use crate::iosurface::Surface;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLCopyAllDevices, MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor,
    MTLTextureUsage,
};
use std::io;

/// A Metal device, as objc2-metal (and wgpu-hal's Metal backend) holds one.
pub type Device = ProtocolObject<dyn MTLDevice>;
/// A Metal texture.
pub type Texture = Retained<ProtocolObject<dyn MTLTexture>>;

/// Every Metal device's `registryID` (one on Apple silicon; an Intel Mac
/// may have two GPUs, and an eGPU).
pub fn registry_ids() -> Vec<u64> {
    MTLCopyAllDevices()
        .iter()
        .map(|device| device.registryID())
        .collect()
}

/// The name of the Metal device with `registry_id` (wgpu's adapter name for
/// it), if there is one.
pub fn device_name(registry_id: u64) -> Option<String> {
    MTLCopyAllDevices()
        .iter()
        .find(|device| device.registryID() == registry_id)
        .map(|device| device.name().to_string())
}

/// `surface` as a BGRA8 unorm texture on `device`: render target, sampled,
/// viewable as sRGB (`PixelFormatView`). Shared storage where the device's
/// memory is unified (Apple silicon), managed elsewhere (as IOSurface-backed
/// textures must be there). The texture keeps the surface alive.
pub fn texture(device: &Device, surface: &Surface) -> io::Result<Texture> {
    // Safety: a plain 2D descriptor of the surface's size.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            surface.width() as usize,
            surface.height() as usize,
            false,
        )
    };
    descriptor.setUsage(
        MTLTextureUsage::RenderTarget
            | MTLTextureUsage::ShaderRead
            | MTLTextureUsage::PixelFormatView,
    );
    descriptor.setStorageMode(if device.hasUnifiedMemory() {
        MTLStorageMode::Shared
    } else {
        MTLStorageMode::Managed
    });
    device
        .newTextureWithDescriptor_iosurface_plane(&descriptor, surface.surface(), 0)
        .ok_or_else(|| io::Error::other("a Metal texture over the IOSurface couldn't be made"))
}
