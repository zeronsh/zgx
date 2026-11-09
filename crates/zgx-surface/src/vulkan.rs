//! Vulkan images shared between two processes on one GPU, with ash: the
//! host makes an image whose memory it exports as an opaque file
//! descriptor (`export_image`), the client makes the same image and imports
//! that memory into it (`import_image`). Both must be on the same physical
//! device and driver (`device_ids` on each side, compared).
//!
//! Needs `VK_KHR_external_memory_fd` on both devices (wgpu-hal 29 enables
//! it where the driver has it) and Vulkan 1.1.
use crate::protocol::{DeviceIds, VulkanImage};
use ash::vk;

/// The image every shared buffer is: BGRA8 unorm, viewable as sRGB too,
/// drawn into (a render pass, a copy) and sampled.
pub const FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;
/// The formats it may be viewed as.
pub const VIEW_FORMATS: [vk::Format; 2] = [vk::Format::B8G8R8A8_UNORM, vk::Format::B8G8R8A8_SRGB];
/// What it is used for, on either side.
pub fn usage() -> vk::ImageUsageFlags {
    vk::ImageUsageFlags::COLOR_ATTACHMENT
        | vk::ImageUsageFlags::SAMPLED
        | vk::ImageUsageFlags::TRANSFER_SRC
        | vk::ImageUsageFlags::TRANSFER_DST
}

/// A physical device's ids (`VkPhysicalDeviceIDProperties`).
///
/// # Safety
/// `physical` is one of `instance`'s, which is Vulkan 1.1 or later.
pub unsafe fn device_ids(instance: &ash::Instance, physical: vk::PhysicalDevice) -> DeviceIds {
    let mut ids = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut ids);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    DeviceIds {
        device_uuid: ids.device_uuid,
        driver_uuid: ids.driver_uuid,
    }
}

/// Whether `physical` can export and import `FORMAT` images of this usage
/// as opaque file descriptors, and whether they must be dedicated.
///
/// # Safety
/// `physical` is one of `instance`'s.
#[cfg(unix)]
pub unsafe fn opaque_fd_support(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Option<bool> {
    let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(FORMAT)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage())
        .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
        .push_next(&mut external);
    let mut external_properties = vk::ExternalImageFormatProperties::default();
    let mut properties = vk::ImageFormatProperties2::default().push_next(&mut external_properties);
    unsafe {
        instance.get_physical_device_image_format_properties2(physical, &info, &mut properties)
    }
    .ok()?;
    let features = external_properties
        .external_memory_properties
        .external_memory_features;
    let both =
        vk::ExternalMemoryFeatureFlags::EXPORTABLE | vk::ExternalMemoryFeatureFlags::IMPORTABLE;
    features
        .contains(both)
        .then(|| features.contains(vk::ExternalMemoryFeatureFlags::DEDICATED_ONLY))
}

/// The image's create info as `desc` says, chained to `external` and
/// `list` (which the caller keeps alive).
fn create_info<'a>(
    desc: &VulkanImage,
    formats: &'a [vk::Format],
    external: &'a mut vk::ExternalMemoryImageCreateInfo<'a>,
    list: &'a mut vk::ImageFormatListCreateInfo<'a>,
) -> vk::ImageCreateInfo<'a> {
    let mut info = vk::ImageCreateInfo::default()
        .flags(vk::ImageCreateFlags::from_raw(desc.flags))
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::from_raw(desc.format))
        .extent(vk::Extent3D {
            width: desc.width,
            height: desc.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::from_raw(desc.tiling))
        .usage(vk::ImageUsageFlags::from_raw(desc.usage))
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(external);
    if !formats.is_empty() {
        *list = vk::ImageFormatListCreateInfo::default().view_formats(formats);
        info = info.push_next(list);
    }
    info
}

/// An image of the host's with its memory exported.
#[cfg(unix)]
pub struct Exported {
    /// The image, bound to `memory`: the caller owns both.
    pub image: vk::Image,
    /// Its dedicated memory.
    pub memory: vk::DeviceMemory,
    /// The memory as a file descriptor, for the client.
    pub fd: std::os::fd::OwnedFd,
    /// What the client makes its image as.
    pub desc: VulkanImage,
}

/// Make a `width` by `height` shared image on `device` and export its
/// memory (dedicated, device-local) as an opaque file descriptor.
///
/// # Safety
/// `device` was made from `physical` of `instance` with
/// `VK_KHR_external_memory_fd` enabled.
#[cfg(unix)]
pub unsafe fn export_image(
    instance: &ash::Instance,
    device: &ash::Device,
    physical: vk::PhysicalDevice,
    width: u32,
    height: u32,
) -> Result<Exported, vk::Result> {
    use std::os::fd::FromRawFd;
    let handle_type = vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD;
    let mut desc = VulkanImage {
        format: FORMAT.as_raw(),
        view_formats: VIEW_FORMATS.iter().map(|f| f.as_raw()).collect(),
        flags: vk::ImageCreateFlags::MUTABLE_FORMAT.as_raw(),
        usage: usage().as_raw(),
        tiling: vk::ImageTiling::OPTIMAL.as_raw(),
        width,
        height,
        allocation_size: 0,
        memory_type_index: 0,
        dedicated: true,
        handle_type: handle_type.as_raw(),
    };
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
    let mut list = vk::ImageFormatListCreateInfo::default();
    let image = {
        let info = create_info(&desc, &VIEW_FORMATS, &mut external, &mut list);
        unsafe { device.create_image(&info, None) }?
    };
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    let types = unsafe { instance.get_physical_device_memory_properties(physical) };
    let Some(type_index) = (0..types.memory_type_count).find(|&i| {
        requirements.memory_type_bits & (1 << i) != 0
            && types.memory_types[i as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    }) else {
        unsafe { device.destroy_image(image, None) };
        return Err(vk::Result::ERROR_FEATURE_NOT_PRESENT);
    };
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(handle_type);
    let allocate = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(type_index)
        .push_next(&mut dedicated)
        .push_next(&mut export);
    let memory = match unsafe { device.allocate_memory(&allocate, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe { device.destroy_image(image, None) };
            return Err(error);
        }
    };
    let fail = |error| {
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
        }
        error
    };
    unsafe { device.bind_image_memory(image, memory, 0) }.map_err(fail)?;
    let fds = ash::khr::external_memory_fd::Device::new(instance, device);
    let info = vk::MemoryGetFdInfoKHR::default()
        .memory(memory)
        .handle_type(handle_type);
    let raw = unsafe { fds.get_memory_fd(&info) }.map_err(fail)?;
    desc.allocation_size = requirements.size;
    desc.memory_type_index = type_index;
    Ok(Exported {
        image,
        memory,
        // Safety: vkGetMemoryFdKHR made a new descriptor for the caller.
        fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) },
        desc,
    })
}

/// Make the image `desc` describes on `device` and import `fd`'s memory
/// into it. The image and memory are the caller's to destroy and free.
///
/// # Safety
/// `device` is on the physical device the image was exported from (its
/// `device_ids` match the host's), with `VK_KHR_external_memory_fd`
/// enabled.
#[cfg(unix)]
pub unsafe fn import_image(
    device: &ash::Device,
    desc: &VulkanImage,
    fd: std::os::fd::OwnedFd,
) -> Result<(vk::Image, vk::DeviceMemory), vk::Result> {
    use std::os::fd::IntoRawFd;
    let handle_type = vk::ExternalMemoryHandleTypeFlags::from_raw(desc.handle_type);
    let formats: Vec<vk::Format> = desc
        .view_formats
        .iter()
        .map(|&f| vk::Format::from_raw(f))
        .collect();
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
    let mut list = vk::ImageFormatListCreateInfo::default();
    let image = {
        let info = create_info(desc, &formats, &mut external, &mut list);
        unsafe { device.create_image(&info, None) }?
    };
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    if requirements.size > desc.allocation_size
        || requirements.memory_type_bits & (1 << desc.memory_type_index) == 0
    {
        unsafe { device.destroy_image(image, None) };
        return Err(vk::Result::ERROR_INVALID_EXTERNAL_HANDLE);
    }
    // The import owns the descriptor once it succeeds.
    let raw = fd.into_raw_fd();
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(handle_type)
        .fd(raw);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut allocate = vk::MemoryAllocateInfo::default()
        .allocation_size(desc.allocation_size)
        .memory_type_index(desc.memory_type_index)
        .push_next(&mut import);
    if desc.dedicated {
        allocate = allocate.push_next(&mut dedicated);
    }
    let memory = match unsafe { device.allocate_memory(&allocate, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe {
                device.destroy_image(image, None);
                libc::close(raw);
            }
            return Err(error);
        }
    };
    if let Err(error) = unsafe { device.bind_image_memory(image, memory, 0) } {
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
        }
        return Err(error);
    }
    Ok((image, memory))
}

/// A physical device's adapter LUID (`VkPhysicalDeviceIDProperties`), as
/// the protocol carries it (`Offer::Dxgi`): `None` where the driver has
/// none (not on Windows).
///
/// # Safety
/// `physical` is one of `instance`'s, which is Vulkan 1.1 or later.
pub unsafe fn device_luid(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Option<u64> {
    let mut ids = vk::PhysicalDeviceIDProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut ids);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    (ids.device_luid_valid != 0).then(|| u64::from_le_bytes(ids.device_luid))
}

/// A Vulkan client's import of a `dxgi` buffer (Windows): the image the
/// host's Direct3D 12 texture is (`FORMAT`, viewable as `VIEW_FORMATS`,
/// `usage()`, optimal tiling) made on `device`, bound to the resource's
/// memory (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D12_RESOURCE_BIT`, dedicated).
/// The image and memory are the caller's to destroy and free (wgpu-hal's
/// `texture_from_raw` with `TextureMemory::Dedicated`); the handle may be
/// closed after, as the import holds its own reference.
///
/// # Safety
/// `device` was made from `physical` of `instance`, on the adapter the
/// host's `Welcome` names (`device_luid`), with
/// `VK_KHR_external_memory_win32` enabled (wgpu-hal 29 enables it where the
/// driver has it).
#[cfg(windows)]
pub unsafe fn import_d3d12_image(
    instance: &ash::Instance,
    device: &ash::Device,
    physical: vk::PhysicalDevice,
    width: u32,
    height: u32,
    handle: &crate::Handle,
) -> Result<(vk::Image, vk::DeviceMemory), vk::Result> {
    use std::os::windows::io::AsRawHandle;
    let handle_type = vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE;
    let raw = handle.as_raw_handle() as vk::HANDLE;
    let desc = VulkanImage {
        format: FORMAT.as_raw(),
        view_formats: VIEW_FORMATS.iter().map(|f| f.as_raw()).collect(),
        flags: vk::ImageCreateFlags::MUTABLE_FORMAT.as_raw(),
        usage: usage().as_raw(),
        tiling: vk::ImageTiling::OPTIMAL.as_raw(),
        width,
        height,
        allocation_size: 0,
        memory_type_index: 0,
        dedicated: true,
        handle_type: handle_type.as_raw(),
    };
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
    let mut list = vk::ImageFormatListCreateInfo::default();
    let image = {
        let info = create_info(&desc, &VIEW_FORMATS, &mut external, &mut list);
        unsafe { device.create_image(&info, None) }?
    };
    let fail = |error| {
        unsafe { device.destroy_image(image, None) };
        error
    };
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    let win32 = ash::khr::external_memory_win32::Device::new(instance, device);
    let mut handle_properties = vk::MemoryWin32HandlePropertiesKHR::default();
    unsafe { win32.get_memory_win32_handle_properties(handle_type, raw, &mut handle_properties) }
        .map_err(fail)?;
    let types = unsafe { instance.get_physical_device_memory_properties(physical) };
    let bits = requirements.memory_type_bits & handle_properties.memory_type_bits;
    let Some(type_index) = (0..types.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && types.memory_types[i as usize]
                .property_flags
                .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
    }) else {
        return Err(fail(vk::Result::ERROR_INVALID_EXTERNAL_HANDLE));
    };
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut import = vk::ImportMemoryWin32HandleInfoKHR::default()
        .handle_type(handle_type)
        .handle(raw);
    let allocate = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(type_index)
        .push_next(&mut import)
        .push_next(&mut dedicated);
    let memory = unsafe { device.allocate_memory(&allocate, None) }.map_err(fail)?;
    if let Err(error) = unsafe { device.bind_image_memory(image, memory, 0) } {
        unsafe {
            device.destroy_image(image, None);
            device.free_memory(memory, None);
        }
        return Err(error);
    }
    Ok((image, memory))
}
