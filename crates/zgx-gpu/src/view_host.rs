//! GPU views under Direct3D 11 (`ZGX_D3D11`): a painter (`viewport::Viewport`)
//! draws with wgpu, which there runs on its no-op backend, so its views
//! would draw nothing. They draw on a Direct3D 12 device of their own
//! instead, on the Direct3D 11 device's adapter, made the first time one
//! shows. Each view's texture is a shared Direct3D 12 resource Direct3D 11
//! opens and samples; after a view's work is submitted, Direct3D 12 signals a
//! shared fence that Direct3D 11 waits on before the frame sampling it.
//!
//! Nothing changes for a painter: it draws with `frame.device` and
//! `frame.queue` as anywhere, and keeps nothing device-made across
//! `Viewport::device_changed`.
use crate::GpuError;
use std::cell::Cell;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
use windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURE2D;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

/// The Direct3D 12 device GPU views draw with under Direct3D 11, and the
/// fence the two share.
pub(crate) struct ViewHost {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    fence12: ID3D12Fence,
    fence11: ID3D11Fence,
    context: ID3D11DeviceContext4,
    d3d11: ID3D11Device1,
    signalled: Cell<u64>,
    _instance: wgpu::Instance,
}

fn failed(what: &'static str) -> impl Fn(windows::core::Error) -> GpuError {
    move |error| GpuError(format!("GPU views under Direct3D 11: {what}: {error}"))
}

impl ViewHost {
    /// A Direct3D 12 device on `d3d11`'s adapter, and the fence they share.
    pub(crate) fn new(d3d11: &crate::d3d11_frame::Device) -> Result<Self, GpuError> {
        // SAFETY: Plain DXGI queries on a live device.
        let luid = unsafe {
            let dxgi: IDXGIDevice = d3d11.device.cast().map_err(failed("its DXGI device"))?;
            let adapter = dxgi.GetAdapter().map_err(failed("its adapter"))?;
            adapter
                .GetDesc()
                .map_err(failed("its adapter's description"))?
                .AdapterLuid
        };
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        // As the Direct3D 12 path compiles (`dx12_compilers`): FXC unless
        // the app ships DXC.
        descriptor.backend_options.dx12.shader_compiler = crate::dx12_compilers()
            .into_iter()
            .next()
            .unwrap_or(wgpu::Dx12Compiler::Fxc);
        descriptor.flags = crate::instance_flags(descriptor.flags, !cfg!(debug_assertions));
        let instance = wgpu::Instance::new(descriptor.with_env());
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
            .into_iter()
            .find(|adapter| {
                // SAFETY: Reads the adapter's description.
                unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }.is_some_and(|hal| {
                    // SAFETY: As above.
                    unsafe { hal.raw_adapter().GetDesc1() }.is_ok_and(|desc| {
                        desc.AdapterLuid.LowPart == luid.LowPart
                            && desc.AdapterLuid.HighPart == luid.HighPart
                    })
                })
            })
            .ok_or_else(|| {
                GpuError(
                    "GPU views under Direct3D 11: no Direct3D 12 adapter matches Direct3D 11's"
                        .into(),
                )
            })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("GPU views (Direct3D 11)"),
            required_features: adapter.features() & crate::viewport::view_device_features(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|error| GpuError(format!("GPU views under Direct3D 11: no device: {error}")))?;
        // SAFETY: The hal device is the one just made; the fence is shared
        // with Direct3D 11 through a handle closed once opened.
        let (fence12, fence11, d3d11_device, context) = unsafe {
            let hal = device.as_hal::<wgpu::hal::api::Dx12>().ok_or_else(|| {
                GpuError("GPU views under Direct3D 11: not a Direct3D 12 device".into())
            })?;
            let raw = hal.raw_device();
            let fence12: ID3D12Fence = raw
                .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                .map_err(failed("a fence"))?;
            let handle = raw
                .CreateSharedHandle(&fence12, None, GENERIC_ALL.0, None)
                .map_err(failed("the fence's handle"))?;
            let d3d11_5: ID3D11Device5 = d3d11.device.cast().map_err(failed("Direct3D 11.4"))?;
            let mut fence11: Option<ID3D11Fence> = None;
            let opened = d3d11_5.OpenSharedFence(handle, &mut fence11);
            let _ = CloseHandle(handle);
            opened.map_err(failed("opening the fence"))?;
            let fence11 =
                fence11.ok_or_else(|| GpuError("GPU views under Direct3D 11: no fence".into()))?;
            let context: ID3D11DeviceContext4 = d3d11
                .context
                .cast()
                .map_err(failed("Direct3D 11.3's context"))?;
            let d3d11_device: ID3D11Device1 =
                d3d11.device.cast().map_err(failed("Direct3D 11.1"))?;
            (fence12, fence11, d3d11_device, context)
        };
        Ok(Self {
            device,
            queue,
            fence12,
            fence11,
            context,
            d3d11: d3d11_device,
            signalled: Cell::new(0),
            _instance: instance,
        })
    }

    /// A view's texture, `size` device pixels: a Direct3D 12 resource in a
    /// shared heap, as wgpu's (`crate::FORMAT`, viewable as sRGB), and
    /// Direct3D 11's view of it to sample.
    pub(crate) fn texture(
        &self,
        (width, height): (u32, u32),
    ) -> Result<(wgpu::Texture, crate::d3d11_frame::Surface), GpuError> {
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: u64::from(width),
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            // Typeless: wgpu views it as unorm and as sRGB.
            Format: DXGI_FORMAT_B8G8R8A8_TYPELESS,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            // Simultaneous access: it decays to the common state as each
            // submission ends, so Direct3D 11 reads it with no transition.
            Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            ..Default::default()
        };
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        // SAFETY: A committed resource on the views' device; its shared
        // handle is closed once Direct3D 11 opens it.
        let (resource, texture11) = unsafe {
            let hal = self
                .device
                .as_hal::<wgpu::hal::api::Dx12>()
                .ok_or_else(|| {
                    GpuError("GPU views under Direct3D 11: not a Direct3D 12 device".into())
                })?;
            let raw = hal.raw_device();
            let mut resource: Option<ID3D12Resource> = None;
            raw.CreateCommittedResource(
                &heap,
                D3D12_HEAP_FLAG_SHARED,
                &desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut resource,
            )
            .map_err(failed("a view's texture"))?;
            let resource = resource
                .ok_or_else(|| GpuError("GPU views under Direct3D 11: no texture".into()))?;
            let handle: HANDLE = raw
                .CreateSharedHandle(&resource, None, GENERIC_ALL.0, None)
                .map_err(failed("a view texture's handle"))?;
            let opened = self.d3d11.OpenSharedResource1::<ID3D11Texture2D>(handle);
            let _ = CloseHandle(handle);
            (
                resource,
                opened.map_err(failed("opening a view's texture"))?,
            )
        };
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        // SAFETY: The resource is the views' device's, as described.
        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                crate::FORMAT,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        };
        let srgb = [crate::FORMAT.add_srgb_suffix()];
        // SAFETY: As above, with the usages the resource allows.
        let texture = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("GPU view (shared with Direct3D 11)"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &srgb,
                },
            )
        };
        let view = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                },
            },
        };
        let mut srv = None;
        // SAFETY: A view of the whole opened texture, in its unorm format.
        unsafe {
            self.d3d11
                .CreateShaderResourceView(&texture11, Some(&view), Some(&mut srv))
        }
        .map_err(failed("a view texture's shader resource view"))?;
        Ok((
            texture,
            crate::d3d11_frame::Surface {
                texture: texture11,
                rtv: None,
                srv,
                width,
                height,
            },
        ))
    }

    /// After a view's work is submitted to the views' queue: Direct3D 11
    /// waits for it before anything it records next runs.
    pub(crate) fn hand_over(&self) -> Result<(), GpuError> {
        let value = self.signalled.get() + 1;
        self.signalled.set(value);
        // SAFETY: The views' queue and the fence both APIs share.
        unsafe {
            let hal = self.queue.as_hal::<wgpu::hal::api::Dx12>().ok_or_else(|| {
                GpuError("GPU views under Direct3D 11: not a Direct3D 12 queue".into())
            })?;
            hal.as_raw()
                .Signal(&self.fence12, value)
                .map_err(failed("signalling the fence"))?;
            self.context
                .Wait(&self.fence11, value)
                .map_err(failed("waiting on the fence"))?;
        }
        Ok(())
    }
}
