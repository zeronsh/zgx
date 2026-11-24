//! A Wayland output's size in the compositor's logical pixels, as xdg-output
//! says it: what a window covering it measures (`WindowOptions::
//! covers_display`). winit gives an output's mode, in the output's own
//! pixels, and its scale as `wl_output` says it, a whole number (2 for an
//! output scaled 1.5); and it takes a window's physical size at the scale the
//! window has as it is first configured, 1 where the compositor has said
//! none by then (Hyprland). A window covering a 3840 by 2160 output scaled
//! 1.5 opened 3840 by 2160 logical pixels, half as large again as the
//! output, most of it off it; xdg-output says 2560 by 1440, with the
//! output's rotation too.
use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output::WlOutput, wl_registry::WlRegistry},
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// The size, in logical pixels, of a window covering `monitor` on Wayland;
/// `None` elsewhere, or where the compositor has no xdg-output.
pub(crate) fn covering_size(
    event_loop: &ActiveEventLoop,
    monitor: &MonitorHandle,
) -> Option<winit::dpi::LogicalSize<u32>> {
    use winit::platform::wayland::MonitorHandleExtWayland;
    let RawDisplayHandle::Wayland(handle) = event_loop.display_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit's display, connected while its event loop runs, which
    // outlives this call.
    let (width, height) = unsafe { logical_size(handle.display, monitor.native_id()) }?;
    Some(winit::dpi::LogicalSize::new(width, height))
}

/// The logical size of the output whose `wl_output` global is `name` (as
/// winit's `native_id` is), asked on a queue of this call's own on
/// `display`.
///
/// # Safety
///
/// `display` is a connected `wl_display` until this returns. Nothing made
/// here holds it after: the backend made on it goes when this returns
/// (destroying the proxies it made, which libwayland then discards what the
/// compositor sends), and the output's objects are destroyed or released
/// before.
unsafe fn logical_size(display: NonNull<c_void>, name: u32) -> Option<(u32, u32)> {
    // SAFETY: a connected display, until this returns (the caller's word),
    // and the backend and its clones go before then.
    let backend = unsafe {
        wayland_backend::sys::client::Backend::from_foreign_display(display.as_ptr().cast())
    };
    let connection = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<LogicalSize>(&connection).ok()?;
    let qh = queue.handle();
    let manager: ZxdgOutputManagerV1 = globals.bind(&qh, 1..=3, ()).ok()?;
    let version = globals.contents().with_list(|list| {
        list.iter()
            .find(|global| global.name == name && global.interface == WlOutput::interface().name)
            .map(|global| global.version)
    });
    let Some(version) = version else {
        manager.destroy();
        return None;
    };
    // (Version 3 on, it can be released.)
    let output: WlOutput = globals.registry().bind(name, version.min(4), &qh, ());
    let xdg_output = manager.get_xdg_output(&output, &qh, ());
    let mut size = LogicalSize::default();
    // The compositor says the size as the object is made, before it answers
    // the roundtrip's sync.
    let answered = queue.roundtrip(&mut size).is_ok();
    xdg_output.destroy();
    manager.destroy();
    if output.version() >= 3 {
        output.release();
    }
    answered.then_some(size.0).flatten()
}

/// What xdg-output says of an output's logical size.
#[derive(Default)]
struct LogicalSize(Option<(u32, u32)>);

impl Dispatch<ZxdgOutputV1, ()> for LogicalSize {
    fn event(
        size: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_output_v1::Event::LogicalSize { width, height } = event {
            size.0 = u32::try_from(width)
                .ok()
                .zip(u32::try_from(height).ok())
                .filter(|&(width, height)| width > 0 && height > 0);
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for LogicalSize {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for LogicalSize {
    // Its mode, scale and name, which winit has.
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for LogicalSize {
    fn event(
        _: &mut Self,
        _: &ZxdgOutputManagerV1,
        _: <ZxdgOutputManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
