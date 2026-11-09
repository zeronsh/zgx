//! Direct3D 12 textures shared between two processes on one adapter: the
//! host makes a committed resource on a shared heap and an NT handle to it
//! (`export_resource`), the transport duplicates the handle into the
//! client, which opens it on its own device (`open_resource`). Both must be
//! on the same adapter, named by its LUID (`adapter_luid` on each side,
//! compared).
//!
//! Every buffer is a 2D `B8G8R8A8_TYPELESS` texture (viewed as BGRA8 unorm,
//! or sRGB), one mip, a render target, with simultaneous access: its state
//! decays to common after each submission on either side, so neither
//! process's wgpu needs to know what the other's last barrier left (and the
//! driver keeps it uncompressed, readable by both).
//!
//! The functions here take and give `windows` 0.62 interfaces (wgpu-hal
//! 29's); `raw` has the same as COM pointers, for a client whose wgpu-hal
//! is on another `windows`.
use crate::Handle;
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle},
};
use windows::{
    Win32::{
        Foundation::{GENERIC_ALL, HANDLE, LUID},
        Graphics::{
            Direct3D12::{
                D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_HEAP_FLAG_SHARED, D3D12_HEAP_PROPERTIES,
                D3D12_HEAP_TYPE_DEFAULT, D3D12_MEMORY_POOL_UNKNOWN, D3D12_RESOURCE_DESC,
                D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS, D3D12_RESOURCE_STATE_COMMON,
                D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12Device, ID3D12Resource,
            },
            Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_SAMPLE_DESC},
        },
    },
    core::PCWSTR,
};

/// The protocol's form of a LUID (`Offer::Dxgi`, `Welcome::adapter`): its
/// high part above its low part.
pub fn luid_to_u64(luid: LUID) -> u64 {
    (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart)
}

/// The LUID of the adapter `device` is on.
pub fn adapter_luid(device: &ID3D12Device) -> u64 {
    // Safety: a plain query of a live device.
    luid_to_u64(unsafe { device.GetAdapterLuid() })
}

fn os(error: windows::core::Error) -> io::Error {
    io::Error::other(format!("Direct3D 12: {error}"))
}

/// A `width` by `height` shared texture on `device` and an NT handle to it,
/// for the client: the resource stays the caller's (wgpu's, once wrapped).
pub fn export_resource(
    device: &ID3D12Device,
    width: u32,
    height: u32,
) -> io::Result<(ID3D12Resource, Handle)> {
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 0,
        VisibleNodeMask: 0,
    };
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: u64::from(width),
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        // Typeless: viewed as unorm and as sRGB, as wgpu-hal makes its own
        // textures with view formats.
        Format: DXGI_FORMAT_B8G8R8A8_TYPELESS,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
            | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
    };
    let mut resource: Option<ID3D12Resource> = None;
    // Safety: the descriptions are complete and outlive the call.
    unsafe {
        device.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_SHARED,
            &desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut resource,
        )
    }
    .map_err(os)?;
    let resource = resource.ok_or_else(|| io::Error::other("Direct3D 12 made no resource"))?;
    // Safety: a resource of this device, on a shared heap.
    let handle =
        unsafe { device.CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null()) }
            .map_err(os)?;
    // Safety: CreateSharedHandle made a new handle for the caller.
    Ok((resource, unsafe { Handle::from_raw_handle(handle.0) }))
}

/// The texture `handle` names (as the host made it, `export_resource`),
/// opened on `device`. The handle may be closed after: the resource holds
/// its own reference.
///
/// # Safety
/// `device` is on the adapter the host's `Welcome` names.
pub unsafe fn open_resource(device: &ID3D12Device, handle: &Handle) -> io::Result<ID3D12Resource> {
    let mut resource: Option<ID3D12Resource> = None;
    // Safety: a handle this process owns, to a shared resource.
    unsafe { device.OpenSharedHandle(HANDLE(handle.as_raw_handle()), &mut resource) }
        .map_err(os)?;
    resource.ok_or_else(|| io::Error::other("Direct3D 12 opened no resource"))
}

/// The same as COM pointers (`ID3D12Device*`, `ID3D12Resource*`), for a
/// client whose wgpu-hal is on another `windows` crate: `Interface::as_raw`
/// of its device in, `Interface::from_raw` of the resource out.
pub mod raw {
    use super::*;
    use std::ffi::c_void;
    use windows::core::Interface;

    /// `adapter_luid` of an `ID3D12Device*`.
    ///
    /// # Safety
    /// `device` is a live `ID3D12Device`.
    pub unsafe fn adapter_luid(device: *mut c_void) -> u64 {
        // Safety: borrowed, not released.
        let device = unsafe { ID3D12Device::from_raw_borrowed(&device) };
        device.map_or(0, super::adapter_luid)
    }

    /// `export_resource` on an `ID3D12Device*`: the resource's pointer holds
    /// a reference the caller owns.
    ///
    /// # Safety
    /// `device` is a live `ID3D12Device`.
    pub unsafe fn export_resource(
        device: *mut c_void,
        width: u32,
        height: u32,
    ) -> io::Result<(*mut c_void, Handle)> {
        // Safety: borrowed, not released.
        let device = unsafe { ID3D12Device::from_raw_borrowed(&device) }
            .ok_or_else(|| io::Error::other("no device"))?;
        let (resource, handle) = super::export_resource(device, width, height)?;
        Ok((resource.into_raw(), handle))
    }

    /// `open_resource` on an `ID3D12Device*`: the resource's pointer holds
    /// a reference the caller owns.
    ///
    /// # Safety
    /// As `open_resource`; `device` is a live `ID3D12Device`.
    pub unsafe fn open_resource(device: *mut c_void, handle: &Handle) -> io::Result<*mut c_void> {
        // Safety: borrowed, not released.
        let device = unsafe { ID3D12Device::from_raw_borrowed(&device) }
            .ok_or_else(|| io::Error::other("no device"))?;
        Ok(unsafe { super::open_resource(device, handle) }?.into_raw())
    }
}
