//! The GPU a Linux display server composites with, so a window is drawn on
//! it (`GpuContext::create`). wgpu's low-power preference takes an
//! integrated GPU wherever there is one, and on a desktop whose displays are
//! on a card beside it every frame was then drawn on the integrated GPU and
//! handed across to the card: Hyprland on an RTX 3080 showed a window drawn
//! on a Ryzen 9800X3D's integrated Radeon (RADV) black, as it shows
//! `vkcube --gpu_number` on that GPU. On a laptop whose integrated GPU drives
//! the displays this chooses what low power did.
//!
//! A Wayland compositor says which device it reads its clients' buffers with
//! (linux-dmabuf's main device, version 4 on); an X server opens its own for
//! a client that asks (DRI3), Xwayland its compositor's. Either is a device
//! number, which can't be compared with another (a card's primary and render
//! nodes have different ones): sysfs gives its PCI address, which Vulkan
//! adapters report (`VK_EXT_pci_bus_info`), and its vendor and device IDs,
//! the same for two cards of a model.
use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_dmabuf_feedback_v1::{self, ZwpLinuxDmabufFeedbackV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// The GPU a display server composites with, as sysfs describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplayGpu {
    /// Its PCI address (`0000:01:00.0`), as wgpu reports a Vulkan adapter's.
    pci_bus_id: String,
    /// Its PCI vendor and device IDs.
    ids: Option<(u32, u32)>,
}

impl DisplayGpu {
    /// The GPU of the display server `window` is shown by: `None` where the
    /// server doesn't say, or its device isn't on PCI.
    pub(crate) fn of_window(window: &winit::window::Window) -> Option<Self> {
        let device = match window.display_handle().ok()?.as_raw() {
            // SAFETY: winit keeps its connection while `window` lives, which
            // outlives this call.
            RawDisplayHandle::Wayland(handle) => unsafe { wayland_main_device(handle.display) },
            RawDisplayHandle::Xlib(handle) => x11_device(usize::try_from(handle.screen).ok()),
            RawDisplayHandle::Xcb(handle) => x11_device(usize::try_from(handle.screen).ok()),
            _ => None,
        }?;
        Self::of_device(Path::new("/sys"), device)
    }

    /// The GPU of device number `device`, as sysfs (mounted at `sys`)
    /// describes it.
    fn of_device(sys: &Path, device: libc::dev_t) -> Option<Self> {
        let (major, minor) = (libc::major(device), libc::minor(device));
        let pci = sys.join(format!("dev/char/{major}:{minor}/device"));
        // A device elsewhere (a platform device on an SoC, a USB display
        // adapter) has no address an adapter reports.
        let subsystem = std::fs::canonicalize(pci.join("subsystem")).ok()?;
        if subsystem.file_name()? != "pci" {
            return None;
        }
        let pci_bus_id = std::fs::canonicalize(&pci)
            .ok()?
            .file_name()?
            .to_str()?
            .to_owned();
        let id = |name: &str| {
            let text = std::fs::read_to_string(pci.join(name)).ok()?;
            u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
        };
        Some(Self {
            pci_bus_id,
            ids: id("vendor").zip(id("device")),
        })
    }

    /// Which of `adapters` draws on this GPU: the one at its PCI address,
    /// else, of those that report none (a driver without
    /// `VK_EXT_pci_bus_info`), the one with its IDs if no other has them.
    pub(crate) fn adapter(&self, adapters: &[wgpu::AdapterInfo]) -> Option<usize> {
        if let Some(at) = adapters.iter().position(|adapter| {
            !adapter.device_pci_bus_id.is_empty()
                && adapter
                    .device_pci_bus_id
                    .eq_ignore_ascii_case(&self.pci_bus_id)
        }) {
            return Some(at);
        }
        let (vendor, device) = self.ids?;
        let mut alike = adapters.iter().enumerate().filter(|(_, adapter)| {
            adapter.device_pci_bus_id.is_empty()
                && adapter.vendor == vendor
                && adapter.device == device
        });
        let (at, _) = alike.next()?;
        alike.next().is_none().then_some(at)
    }
}

/// The device a Wayland compositor reads its clients' buffers with
/// (linux-dmabuf's main device), asked on a queue of this call's own on
/// `display`.
///
/// # Safety
///
/// `display` is a connected `wl_display` until this returns. Nothing made
/// here holds it after: the backend made on it goes when this returns
/// (destroying the proxies it made, which libwayland then discards what the
/// compositor sends), and the dmabuf objects are destroyed before.
unsafe fn wayland_main_device(display: NonNull<c_void>) -> Option<libc::dev_t> {
    // SAFETY: a connected display, until this returns (the caller's word),
    // and the backend and its clones go before then.
    let backend = unsafe {
        wayland_backend::sys::client::Backend::from_foreign_display(display.as_ptr().cast())
    };
    let connection = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<MainDevice>(&connection).ok()?;
    // Version 6 says no main device (a tranche says it, with the sampling
    // flag); 4 and 5 do, and a compositor serves any version up to its own.
    let dmabuf: ZwpLinuxDmabufV1 = globals.bind(&queue.handle(), 4..=5, ()).ok()?;
    let feedback = dmabuf.get_default_feedback(&queue.handle(), ());
    let mut state = MainDevice::default();
    // The compositor sends the feedback as the object is made, before it
    // answers the roundtrip's sync.
    let answered = queue.roundtrip(&mut state).is_ok();
    feedback.destroy();
    dmabuf.destroy();
    answered.then_some(state.device).flatten()
}

/// What a compositor's default dmabuf feedback says of its main device.
#[derive(Default)]
struct MainDevice {
    device: Option<libc::dev_t>,
}

/// A `dev_t` as linux-dmabuf sends one: its bytes, in the machine's order.
fn device_number(bytes: &[u8]) -> Option<libc::dev_t> {
    Some(libc::dev_t::from_ne_bytes(bytes.try_into().ok()?))
}

impl Dispatch<WlRegistry, GlobalListContents> for MainDevice {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for MainDevice {
    // Formats and modifiers, which versions from 4 on don't send.
    fn event(
        _: &mut Self,
        _: &ZwpLinuxDmabufV1,
        _: <ZwpLinuxDmabufV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for MainDevice {
    fn event(
        state: &mut Self,
        _: &ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // (The format table's file closes as its event drops.)
        if let zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } = event {
            state.device = device_number(&device);
        }
    }
}

/// The device an X server draws with, which it opens for a client that asks
/// (DRI3, as Mesa's drivers ask it), on `screen` (else the display's own).
fn x11_device(screen: Option<usize>) -> Option<libc::dev_t> {
    use std::os::unix::fs::MetadataExt;
    use x11rb::{connection::Connection as _, protocol::dri3::ConnectionExt as _};
    let (connection, default_screen) = x11rb::connect(None).ok()?;
    let root = connection
        .setup()
        .roots
        .get(screen.unwrap_or(default_screen))?
        .root;
    connection.dri3_query_version(1, 0).ok()?.reply().ok()?;
    let opened = connection.dri3_open(root, x11rb::NONE).ok()?.reply().ok()?;
    let device = std::fs::File::from(opened.device_fd).metadata().ok()?;
    Some(device.rdev())
}

#[cfg(test)]
mod tests {
    use super::{DisplayGpu, device_number};
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    /// A sysfs of two GPUs as the one this was found on has them: a Ryzen's
    /// integrated Radeon (card1, renderD128) and an RTX 3080 (card2,
    /// renderD129), and a platform device's node (226:3).
    fn sysfs(name: &str) -> PathBuf {
        let sys =
            std::env::temp_dir().join(format!("zgx-display-gpu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sys);
        let mkdir = |path: &str| std::fs::create_dir_all(sys.join(path)).unwrap();
        let write = |path: &str, text: &str| std::fs::write(sys.join(path), text).unwrap();
        let link = |path: &str, target: &str| symlink(target, sys.join(path)).unwrap();
        mkdir("bus/pci");
        mkdir("bus/platform");
        mkdir("dev/char");
        for (pci, vendor, device, nodes) in [
            (
                "pci0000:00/0000:00:08.1/0000:0b:00.0",
                "0x1002",
                "0x13c0",
                [("card1", "226:1"), ("renderD128", "226:128")],
            ),
            (
                "pci0000:00/0000:00:01.1/0000:01:00.0",
                "0x10de",
                "0x2216",
                [("card2", "226:2"), ("renderD129", "226:129")],
            ),
        ] {
            let address = pci.rsplit('/').next().unwrap();
            mkdir(&format!("devices/{pci}"));
            write(&format!("devices/{pci}/vendor"), &format!("{vendor}\n"));
            write(&format!("devices/{pci}/device"), &format!("{device}\n"));
            link(&format!("devices/{pci}/subsystem"), "../../../../bus/pci");
            for (node, number) in nodes {
                mkdir(&format!("devices/{pci}/drm/{node}"));
                link(
                    &format!("devices/{pci}/drm/{node}/device"),
                    &format!("../../../{address}"),
                );
                link(
                    &format!("dev/char/{number}"),
                    &format!("../../devices/{pci}/drm/{node}"),
                );
            }
        }
        mkdir("devices/platform/fd4ab000.gpu/drm/card3");
        link(
            "devices/platform/fd4ab000.gpu/subsystem",
            "../../../bus/platform",
        );
        link(
            "devices/platform/fd4ab000.gpu/drm/card3/device",
            "../../../fd4ab000.gpu",
        );
        link(
            "dev/char/226:3",
            "../../devices/platform/fd4ab000.gpu/drm/card3",
        );
        sys
    }

    fn gpu(sys: &Path, major: u32, minor: u32) -> Option<DisplayGpu> {
        DisplayGpu::of_device(sys, libc::makedev(major, minor))
    }

    fn adapter(name: &str, pci_bus_id: &str, vendor: u32, device: u32) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: name.into(),
            vendor,
            device,
            device_type: wgpu::DeviceType::Other,
            device_pci_bus_id: pci_bus_id.into(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 4,
            subgroup_max_size: 128,
            transient_saves_memory: false,
        }
    }

    #[test]
    fn a_device_number_is_the_gpu_at_its_pci_address() {
        let sys = sysfs("address");
        let card = DisplayGpu {
            pci_bus_id: "0000:01:00.0".into(),
            ids: Some((0x10de, 0x2216)),
        };
        // A compositor may say either node of the card: both are the card.
        assert_eq!(gpu(&sys, 226, 129), Some(card.clone()));
        assert_eq!(gpu(&sys, 226, 2), Some(card));
        assert_eq!(gpu(&sys, 226, 128).unwrap().pci_bus_id, "0000:0b:00.0");
        // Not on PCI, or no such device: no GPU said.
        assert_eq!(gpu(&sys, 226, 3), None);
        assert_eq!(gpu(&sys, 226, 130), None);
        std::fs::remove_dir_all(sys).unwrap();
    }

    #[test]
    fn the_adapter_at_the_display_gpus_address_draws() {
        let card = DisplayGpu {
            pci_bus_id: "0000:01:00.0".into(),
            ids: Some((0x10de, 0x2216)),
        };
        let radeon = adapter(
            "AMD Ryzen 7 9800X3D (RADV RAPHAEL_MENDOCINO)",
            "0000:0b:00.0",
            0x1002,
            0x13c0,
        );
        let nvidia = adapter("NVIDIA GeForce RTX 3080", "0000:01:00.0", 0x10de, 0x2216);
        let llvmpipe = adapter("llvmpipe (LLVM 21.1.8, 256 bits)", "", 0x10005, 0);
        assert_eq!(
            card.adapter(&[radeon.clone(), nvidia.clone(), llvmpipe.clone()]),
            Some(1)
        );
        assert_eq!(card.adapter(&[nvidia.clone(), radeon.clone()]), Some(0));
        // The display's GPU has no driver here: none is chosen for it.
        assert_eq!(card.adapter(&[radeon.clone(), llvmpipe]), None);
        // Another card of the same model is another card.
        let twin = adapter("NVIDIA GeForce RTX 3080", "0000:02:00.0", 0x10de, 0x2216);
        assert_eq!(card.adapter(&[twin.clone(), nvidia]), Some(1));
        assert_eq!(card.adapter(&[twin, radeon]), None);
    }

    #[test]
    fn a_driver_that_reports_no_address_is_matched_by_its_ids_alone() {
        let card = DisplayGpu {
            pci_bus_id: "0000:01:00.0".into(),
            ids: Some((0x10de, 0x2216)),
        };
        let unaddressed = adapter("NVIDIA GeForce RTX 3080", "", 0x10de, 0x2216);
        let radeon = adapter("AMD Radeon", "0000:0b:00.0", 0x1002, 0x13c0);
        assert_eq!(
            card.adapter(&[radeon.clone(), unaddressed.clone()]),
            Some(1)
        );
        // Two such, and it can't be told which: neither.
        assert_eq!(
            card.adapter(&[unaddressed.clone(), radeon, unaddressed]),
            None
        );
    }

    #[test]
    fn linux_dmabuf_sends_a_device_number_in_the_machines_byte_order() {
        let number = libc::makedev(226, 129);
        assert_eq!(device_number(&number.to_ne_bytes()), Some(number));
        assert_eq!(device_number(&[0; 4]), None);
        assert_eq!(device_number(&[]), None);
    }
}
