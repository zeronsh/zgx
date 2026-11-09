//! A client (or host) on wgpu 29: its adapter matched to the host's GPU,
//! shared buffers as `wgpu::Texture`s, and a readback into shared memory
//! where it can't share.
//!
//! A client on another wgpu (Horizon is on 30) copies this module and
//! changes `wgpu` to its own, checking that what it reaches of wgpu-hal
//! (`as_hal`, `texture_from_raw` with `TextureMemory::Dedicated`,
//! `enabled_device_extensions`, `create_texture_from_hal`) has the same
//! shape there and that its ash is this crate's (0.38).
use crate::{
    Handle,
    protocol::{Buffer, BufferDesc, DeviceIds, Offer},
};
use std::io;

/// The wgpu these helpers take: a client on wgpu 29 uses its own.
pub use wgpu;

/// The shared buffers' format (BGRA8 unorm, premultiplied).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// The other format they may be viewed as: sRGB, to shade in linear light.
pub const VIEW_FORMATS: [wgpu::TextureFormat; 1] = [wgpu::TextureFormat::Bgra8UnormSrgb];
/// What a shared texture may be used for, on either side.
pub const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

fn unsupported(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, why.to_owned())
}

/// The ids of a Vulkan adapter's physical device; `None` on other
/// backends.
pub fn device_ids(adapter: &wgpu::Adapter) -> Option<DeviceIds> {
    #[cfg(target_os = "linux")]
    {
        // Safety: only read through.
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let instance = hal.shared_instance().raw_instance();
        // Safety: the adapter's own physical device, on wgpu's 1.1+ instance.
        Some(unsafe { crate::vulkan::device_ids(instance, hal.raw_physical_device()) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = adapter;
        None
    }
}

/// What to offer a host for `adapters` (`Instance::enumerate_adapters`):
/// Vulkan images on any of their devices, if any is Vulkan.
pub fn vulkan_offer(adapters: &[wgpu::Adapter]) -> Option<Offer> {
    let devices: Vec<DeviceIds> = adapters.iter().filter_map(device_ids).collect();
    (!devices.is_empty()).then_some(Offer::Vulkan { devices })
}

/// The adapter among `adapters` on the device `ids` names: a Vulkan
/// device by its UUIDs, or a Metal one (`DeviceIds::metal`, an `IoSurface`
/// welcome's) by its name, which a device made on it then confirms
/// (`iosurface_ids_of`).
pub fn adapter_for(adapters: Vec<wgpu::Adapter>, ids: &DeviceIds) -> Option<wgpu::Adapter> {
    if let Some(registry_id) = ids.metal_registry_id() {
        #[cfg(target_os = "macos")]
        {
            let name = crate::metal::device_name(registry_id)?;
            return adapters.into_iter().find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Metal && info.name == name
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = registry_id;
            return None;
        }
    }
    adapters
        .into_iter()
        .find(|adapter| device_ids(adapter).as_ref() == Some(ids))
}

/// What to offer a host for `adapters`: IOSurfaces drawn on any of the
/// system's Metal devices, if any adapter is Metal (macOS).
pub fn iosurface_offer(adapters: &[wgpu::Adapter]) -> Option<Offer> {
    #[cfg(target_os = "macos")]
    {
        let metal = adapters
            .iter()
            .any(|a| a.get_info().backend == wgpu::Backend::Metal);
        let devices = crate::metal::registry_ids();
        (metal && !devices.is_empty()).then_some(Offer::IoSurface { devices })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = adapters;
        None
    }
}

/// Whether `device` can share IOSurfaces: it is Metal (macOS).
pub fn can_share_iosurface(device: &wgpu::Device) -> bool {
    iosurface_ids_of(device).is_some()
}

/// `device`'s Metal device as `DeviceIds::metal`, if it is Metal.
pub fn iosurface_ids_of(device: &wgpu::Device) -> Option<DeviceIds> {
    #[cfg(target_os = "macos")]
    {
        use objc2_metal::MTLDevice as _;
        // Safety: only read through.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
        Some(DeviceIds::metal(hal.raw_device().registryID()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = device;
        None
    }
}

/// A Metal texture over an IOSurface as a wgpu texture on `device`
/// (`FORMAT`, `USAGE`, viewable as `VIEW_FORMATS`).
#[cfg(target_os = "macos")]
fn wrap_metal(
    device: &wgpu::Device,
    metal: crate::metal::Texture,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    // Safety: Metal made the texture on this device as `metal::texture`
    // describes: 2D, one layer and mip level, `FORMAT`, this size.
    let raw = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            metal,
            FORMAT,
            objc2_metal::MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
        )
    };
    // Safety: made from this device's raw device, its usage covering
    // `USAGE` and its sRGB views (`PixelFormatView`).
    unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Metal>(
            raw,
            &wgpu::TextureDescriptor {
                label: Some("zgx surface"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: USAGE,
                view_formats: &VIEW_FORMATS,
            },
        )
    }
}

/// An IOSurface of `width` by `height` as a texture on `device`, for
/// another process to draw into: the texture, the surface (for
/// `iosurface::HostChannel::send`) and what the buffer is
/// (`BufferDesc::IoSurface`).
pub fn export_iosurface(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> io::Result<(wgpu::Texture, crate::iosurface::Surface, BufferDesc)> {
    #[cfg(target_os = "macos")]
    {
        let surface = crate::iosurface::Surface::new_bgra(width, height)?;
        let metal = {
            // Safety: the raw device only makes a texture, which wgpu then owns.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or_else(|| unsupported("not a Metal device"))?;
            crate::metal::texture(hal.raw_device(), &surface)?
        };
        let texture = wrap_metal(device, metal, width, height);
        let desc = BufferDesc::IoSurface {
            id: surface.id(),
            surface: None,
        };
        Ok((texture, surface, desc))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (device, width, height);
        Err(unsupported("IOSurfaces are shared on macOS"))
    }
}

/// Whether `device` can share memory with another process
/// (`VK_KHR_external_memory_fd` enabled, its images exportable).
pub fn can_share(device: &wgpu::Device) -> bool {
    #[cfg(target_os = "linux")]
    {
        // Safety: only read through.
        let Some(hal) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
            return false;
        };
        let enabled = hal
            .enabled_device_extensions()
            .contains(&ash::khr::external_memory_fd::NAME);
        let instance = hal.shared_instance().raw_instance();
        // Safety: the device's own physical device.
        enabled
            && unsafe { crate::vulkan::opaque_fd_support(instance, hal.raw_physical_device()) }
                .is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        false
    }
}

/// The ids of `device`'s physical device, if it is Vulkan.
pub fn device_ids_of(device: &wgpu::Device) -> Option<DeviceIds> {
    #[cfg(target_os = "linux")]
    {
        // Safety: only read through.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let instance = hal.shared_instance().raw_instance();
        Some(unsafe { crate::vulkan::device_ids(instance, hal.raw_physical_device()) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        None
    }
}

#[cfg(target_os = "linux")]
fn texture_descs(
    width: u32,
    height: u32,
) -> (
    wgpu::hal::TextureDescriptor<'static>,
    wgpu::TextureDescriptor<'static>,
) {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    (
        wgpu::hal::TextureDescriptor {
            label: Some("zgx surface"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUses::COLOR_TARGET
                | wgpu::TextureUses::RESOURCE
                | wgpu::TextureUses::COPY_SRC
                | wgpu::TextureUses::COPY_DST,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: VIEW_FORMATS.to_vec(),
        },
        wgpu::TextureDescriptor {
            label: Some("zgx surface"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: USAGE,
            view_formats: &VIEW_FORMATS,
        },
    )
}

/// A Vulkan or IOSurface buffer the host sent, as a texture on `device`
/// (`FORMAT`, `USAGE`, viewable as `VIEW_FORMATS`): draw into it, finish
/// the GPU's work, then present it.
///
/// # Safety
/// `device` is on the adapter the host's `Welcome` names (`adapter_for`).
pub unsafe fn import(device: &wgpu::Device, buffer: Buffer) -> io::Result<wgpu::Texture> {
    if let BufferDesc::IoSurface { surface, .. } = &buffer.desc {
        #[cfg(target_os = "macos")]
        {
            let surface = surface
                .as_ref()
                .ok_or_else(|| unsupported("an IOSurface buffer whose surface didn't arrive"))?;
            let metal = {
                // Safety: the raw device only makes a texture, which wgpu then owns.
                let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
                    .ok_or_else(|| unsupported("not a Metal device"))?;
                crate::metal::texture(hal.raw_device(), surface)?
            };
            return Ok(wrap_metal(device, metal, surface.width(), surface.height()));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = surface;
            return Err(unsupported("IOSurfaces are shared on macOS"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let BufferDesc::Vulkan(desc) = &buffer.desc else {
            return Err(unsupported("not a Vulkan buffer"));
        };
        let fd = buffer
            .handle
            .ok_or_else(|| unsupported("a Vulkan buffer without its memory"))?;
        let (hal_desc, desc_wgpu) = texture_descs(desc.width, desc.height);
        let hal_texture = {
            // Safety: used only to make the image, which wgpu then owns.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| unsupported("not a Vulkan device"))?;
            if !hal
                .enabled_device_extensions()
                .contains(&ash::khr::external_memory_fd::NAME)
            {
                return Err(unsupported("VK_KHR_external_memory_fd is not enabled"));
            }
            let (image, memory) =
                unsafe { crate::vulkan::import_image(hal.raw_device(), desc, fd) }
                    .map_err(|e| io::Error::other(format!("importing a Vulkan image: {e}")))?;
            // Safety: the image was made as `hal_desc` says; wgpu-hal owns
            // it and its memory from here.
            unsafe {
                hal.texture_from_raw(
                    image,
                    &hal_desc,
                    None,
                    wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
                )
            }
        };
        Ok(unsafe {
            device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &desc_wgpu)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (device, buffer);
        Err(unsupported("Vulkan buffers are shared on Linux"))
    }
}

/// A texture on `device` whose memory another process can import: the
/// texture, its memory's handle and what the other side makes it as
/// (`BufferDesc::Vulkan`).
pub fn export(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> io::Result<(wgpu::Texture, Handle, BufferDesc)> {
    #[cfg(target_os = "linux")]
    {
        let (hal_desc, desc_wgpu) = texture_descs(width, height);
        let (hal_texture, fd, desc) = {
            // Safety: used only to make the image, which wgpu then owns.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| unsupported("not a Vulkan device"))?;
            let instance = hal.shared_instance().raw_instance();
            // Safety: the device's own instance and physical device; the
            // extension is checked by `can_share`, and creation fails without.
            let exported = unsafe {
                crate::vulkan::export_image(
                    instance,
                    hal.raw_device(),
                    hal.raw_physical_device(),
                    width,
                    height,
                )
            }
            .map_err(|e| io::Error::other(format!("exporting a Vulkan image: {e}")))?;
            let texture = unsafe {
                hal.texture_from_raw(
                    exported.image,
                    &hal_desc,
                    None,
                    wgpu::hal::vulkan::TextureMemory::Dedicated(exported.memory),
                )
            };
            (texture, exported.fd, exported.desc)
        };
        let texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &desc_wgpu)
        };
        Ok((texture, fd, BufferDesc::Vulkan(desc)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (device, width, height);
        Err(unsupported("Vulkan buffers are shared on Linux"))
    }
}

/// A texture read back into shared memory, for a client that can't share
/// GPU memory (`shm`): `copy` it in a frame's commands, submit, then
/// `write` it into the buffer's mapping.
pub struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
}
impl Readback {
    /// For a 4-byte-a-pixel texture of `width` by `height`.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let padded = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zgx surface readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            width,
            height,
            padded,
        }
    }
    /// Copy `texture` (its size, 4 bytes a pixel) into the readback.
    pub fn copy(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }
    /// Wait for the copy submitted, then write its rows into `out`,
    /// `stride` bytes apart.
    pub fn write(
        &self,
        device: &wgpu::Device,
        out: &mut [u8],
        stride: usize,
    ) -> Result<(), wgpu::PollError> {
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;
        {
            let mapped = slice.get_mapped_range();
            let row = (self.width * 4) as usize;
            for y in 0..self.height as usize {
                let from = &mapped[y * self.padded as usize..][..row];
                out[y * stride..][..row].copy_from_slice(from);
            }
        }
        self.buffer.unmap();
        Ok(())
    }
}
