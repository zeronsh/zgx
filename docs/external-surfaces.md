# External surfaces: another program's frames as an element

An external surface shows what another process draws, with GPU work of its own (a game, an emulator, a renderer on another wgpu), as an ordinary element: sized, rounded, clipped, faded, popovers over it. Its input is the program's: keys while it has focus, the pointer over it, and a pointer lock it asks for. On Linux and macOS the frames never touch a CPU: the host allocates the buffers on its GPU, the program draws into them, the host samples them.

```rust
use zgx::compose::prelude::*;
use zgx_desktop::surface::{App, AppOptions, surface};

let app = App::spawn(&cx.ui.runtime, AppOptions::new("horizon-client").args(["--zgx-surface"]))?;
cx.render(surface(&app).w_full().h(600.).rounded(16.).overflow_hidden());
```

```luau
local game = ui.app("game", { command = "horizon-client", args = { "--zgx-surface" } })
return ui.surface { app = game, w = "100%", h = 600, rounded = 16 }
```

`crates/zgx-desktop/examples/external_surface.rs` shows `crates/zgx-surface/examples/spinner_client.rs` (a wgpu 29 client) in a rounded box with zgx UI over it:

```sh
cargo build -p zgx-surface --features wgpu29 --example spinner_client
cargo run -p zgx-desktop --features surfaces --example external_surface        # Vulkan images
cargo run -p zgx-desktop --features surfaces --example external_surface -- --shm   # shared memory
```

## Crates and features

| | |
| --- | --- |
| `zgx-surface` | the protocol, its connection and the client (`Client`), shared memory (`shm`), the host's and client's rings (`ring`). No wgpu. |
| `zgx-surface/vulkan` | ash 0.38 helpers: a device's ids, an exportable image, an import (`vulkan`) |
| `zgx-surface` on macOS | IOSurfaces (`iosurface::Surface`) and the Mach channel their ports travel over (`iosurface::{HostChannel, ClientChannel}`). No Metal. |
| `zgx-surface/metal` | objc2-metal 0.3 helpers (macOS): the Metal devices by `registryID`, an IOSurface as a Metal texture (`metal`) |
| `zgx-surface/wgpu29` | a wgpu 29 client's (and the host's) helpers: the adapter matching the host's GPU, buffers as `wgpu::Texture`s, a readback into shared memory (`wgpu29`) |
| `zgx-surface/host` | the zgx side: `App`, `surface` (re-exported as `zgx_desktop::surface`, the desktop host's `surfaces` feature) |
| `zgx-script/apps` | `ui.app` and `ui.surface` (zgx-run has it) |

## How it works

1. **Start.** The host starts the program (no shell) with one end of a Unix socket pair inherited; `ZGX_SURFACE_FD` names it. On macOS `ZGX_SURFACE_MACH` also names the host's Mach rendezvous, for IOSurfaces ([below](#iosurfaces-and-the-mach-channel)). On Linux the program dies with the host however the host ends (`PR_SET_PDEATHSIG`); closing the session sends `Close` and kills it 1.5 s later if it hasn't exited.
2. **Handshake.** The client says what it can take, best first (`Hello`): Vulkan images on any of its GPUs (by `deviceUUID` and `driverUUID`), IOSurfaces on any of its Metal devices (by `registryID`), or shared memory. The host answers (`Welcome`) with the kind it will send and, for Vulkan or IOSurfaces, its own GPU's ids, which the client draws on (`wgpu29::adapter_for`). The host answers when it first draws the element, as it then knows its device.
3. **Buffers.** The host makes a ring of three buffers at the element's size in device pixels and sends them (`Resize`, then `Buffers`, a generation). A new size makes a new generation once it has held for 120 ms; meanwhile the last frame shows stretched, so a window being dragged doesn't rebuild buffers every frame.
4. **Frames.** The client takes a free buffer (`acquire`), draws into it, **finishes its GPU work** (v1 shares no semaphore: `device.poll` waiting for its submission) and presents it (`Present`). The host's GPU view draws the newest presented buffer over its box (one textured triangle, no CPU) at its next frame. A buffer presented over one not yet shown goes back at once (`Release`); the one showing goes back once a newer shows and the host's GPU work that read it is done (`on_submitted_work_done`, polled until it fires). A client that draws only with two buffers free (`Client::free_buffers() >= 2`) is paced to the frames shown and never draws one replaced unseen.
5. **Idle.** A surface hidden (scrolled away, its window hidden) tells the client (`Visible(false)`): a client then draws nothing until it shows again (zgx's rule: idle windows do no work). An element not invalidated draws nothing: the host draws only when a frame is presented.

### Buffer kinds

- **`vulkan` (Linux):** the host makes each buffer a `VkImage` (`B8G8R8A8_UNORM`, viewable as `_SRGB`, colour attachment, sampled, transfer source and destination; optimal tiling) on dedicated device-local memory exported as an opaque file descriptor (`VK_KHR_external_memory_fd`, which wgpu-hal 29 enables where the driver has it), wraps it as a wgpu texture (`create_texture_from_hal`) and clears it once, so its first use leaves the undefined layout before the client draws. It sends the descriptor with the image's exact parameters (format, view formats, flags, usage, tiling, size, allocation size, memory type, dedicated). The client makes the same image on the same physical device and imports the memory (`wgpu29::import`).
  - Opaque descriptors were chosen over dma-buf with DRM modifiers: on this NVIDIA driver both work, opaque needs no modifier negotiation, and both sides are the same GPU anyway. A dma-buf would be the route to a compositor or a video decoder.
  - **Image layouts and queue ownership are not transferred in v1**: each side's wgpu tracks its own idea of the image's layout. NVIDIA's driver doesn't care; a driver that compresses colour images (AMD's DCC, Intel's CCS) may need the images in the `GENERAL` layout, or explicit acquire and release barriers with `VK_QUEUE_FAMILY_EXTERNAL`, with semaphores (`VK_KHR_external_semaphore_fd`) in place of the client's wait. That is v2.
- **`shm` (everywhere):** each buffer is shared memory (a memfd on Linux, POSIX shared memory elsewhere on Unix, a file mapping on Windows), BGRA8 rows. The client writes pixels (`wgpu29::Readback` reads its frame back into it); the host uploads the newest (`Queue::write_texture`) as it draws. Two CPU copies a frame: the client's readback and the host's upload.
- **`iosurface` (macOS):** the host makes each buffer an `IOSurface` (BGRA, four bytes a pixel, not global), a Metal texture over it on its own device (`newTextureWithDescriptor:iosurface:plane:`; `BGRA8Unorm`, render target, sampled, viewable as sRGB; shared storage on Apple silicon, managed on an Intel Mac's discrete GPU) wrapped as a wgpu texture (`texture_from_raw`, `create_texture_from_hal::<Metal>`), and clears it once. Their Mach ports go to the client over the session's Mach channel, then the `Buffers` naming them by id over the socket; the client makes each port a surface again and a texture on its own Metal device (`wgpu29::import`). Both sides are on the same GPU: the host's `Welcome` names its device's `registryID` (`DeviceIds::metal`), which the client's adapter is picked by (one GPU on Apple silicon; an Intel Mac may have two).
- **`dxgi` (Windows):** in the protocol, not implemented: see [the seam](#for-the-windows-backend).

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| GPU buffers | `vulkan`: Vulkan images, opaque fds over the socket (`SCM_RIGHTS`) | `iosurface`: IOSurfaces, Mach ports over a Mach channel beside the socket | `dxgi`: not yet |
| Same GPU by | `deviceUUID` and `driverUUID` | `registryID` | adapter LUID (planned) |
| Fallback | `shm` (memfd) | `shm` (POSIX shared memory) | `shm` (file mapping) |
| Sync (v1) | the client waits for its submission; the host releases after its work reading it | the same | the same |
| The program dies with the host | `PR_SET_PDEATHSIG` | `Close`, then killed 1.5 s later; not if the host is killed | not yet |
| Checked | run here (`vulkan_share`, the spinner) | compile-checked; `iosurface_share` runs on CI's macOS job | compile-checked |

### IOSurfaces and the Mach channel

A Unix socket carries no Mach ports, so on macOS a session has a Mach channel beside it, set up as Chromium's `MachPortRendezvous` is:

1. The host checks in one bootstrap service for the process (`bootstrap_check_in`, a name with the host's pid and 128 random bits, made as the first program starts) and gives each program it starts the name in `ZGX_SURFACE_MACH`.
2. The client (`Client::connect`, when it offers `IoSurface`) looks the name up (`bootstrap_look_up`) and sends a check-in carrying a send right to a port of its own, before its `Hello`.
3. The rendezvous's thread keeps each check-in by its sender's pid, read from the message's audit token (`MACH_RCV_TRAILER_AUDIT`). A session takes only the check-in of the process it started: another process that found the name reaches no session. A client offering IOSurfaces that didn't check in within 2 s is treated as not offering them.
4. Each generation's surfaces go to the client as one Mach message of their ports (`IOSurfaceCreateMachPort`, moved), its generation and their ids, sent before the socket's `Buffers`. `Client` takes it as that `Buffers` arrives (`IOSurfaceLookupFromMachPort`, matched by id) and fills in each `BufferDesc::IoSurface`'s `surface`.

Not XPC: an anonymous listener's endpoint travels only over XPC, so it would need the same bootstrap name to start from. Not global IOSurface ids (`kIOSurfaceIsGlobal`): deprecated, and any process could open the surfaces by guessing an id. A sandboxed host or client needs the bootstrap lookup allowed (an App Sandbox `mach-lookup` exception for `dev.zgx.surface.`).

## The protocol (version 1)

A frame is its length (`u32`, of what follows it), its kind (`u16`), how many handles ride with it (`u16`) and its payload; little-endian, strings and lists as a `u32` count then items. File descriptors go beside the bytes (`SCM_RIGHTS`); a payload names each by its place among the frame's. A kind the reader doesn't know is skipped. `crates/zgx-surface/src/protocol.rs` is the definition.

| Client → host | |
| --- | --- |
| `Hello { version, offers }` | first; `offers`: `Vulkan { devices: [DeviceIds] }`, `Shm`, `IoSurface { devices: [registryID] }`, `Dxgi { adapters: [LUID] }`, best first |
| `Present { generation, index }` | buffer `index` holds a finished frame: show it |
| `SetCursor(cursor)` | `Default`, `Pointer`, `Text`, `Crosshair`, `Move`, `Grab`, `Grabbing`, `NotAllowed`, `Wait`, `Progress`, `Help`, `EwResize`, `NsResize`, `NeswResize`, `NwseResize`, `Hidden` (shown as the arrow while unlocked: zgx has no hidden cursor yet) |
| `LockPointer(bool)` | lock the pointer (granted only within a second of a press or key, as the web's), or let it go |
| `Title(string)` | |

| Host → client | |
| --- | --- |
| `Welcome { version, kind, device }` | how buffers are shared; `device`: the host's GPU's ids for Vulkan, its Metal device for IOSurfaces (`DeviceIds::metal(registryID)`) |
| `Buffers(Generation { generation, width, height, format, buffers })` | a new ring; each buffer `{ index, handle, desc }`, `desc` `Shm { offset, stride, len }`, `Vulkan(VulkanImage)` or `IoSurface { id }` (its port over the Mach channel) |
| `Release { generation, index }` | the host no longer reads it |
| `Resize { width, height, scale }` | the element's size in device pixels, and its scale |
| `Visible(bool)` | whether any of it shows (it is shown until told otherwise) |
| `Focus(bool)` | |
| `Key(KeyEvent { code, down, repeat, modifiers })` | `code` as the DOM's `KeyboardEvent.code` (`"KeyW"`, `"Space"`, `"ShiftLeft"`): the physical key, whatever the layout |
| `Text(string)` | typed text or an input method's commit, apart from keys |
| `PointerMove { x, y }` | device pixels from its top left |
| `PointerMotion { dx, dy }` | raw motion while locked (about a point a count) |
| `Button { button, down, x, y }` | `Left`, `Right`, `Middle`, `Back`, `Forward`, `Other(n)` |
| `Wheel { dx, dy, unit }` | positive `dy` scrolls down, as the DOM's `deltaY`; zgx sends `Pixels` (device pixels), `Lines` is reserved |
| `PointerLock(bool)` | the lock began or ended (the client asked, Escape, a lost focus) |
| `PointerLeave` | |
| `Close` | finish and exit |

Keys reach the program while the element has focus (a press focuses it); Command or Control chords go on to the app's shortcuts too. Escape during a lock ends the lock and reaches no one, as zgx's pointer lock does everywhere.

## A client in about fifty lines of wgpu

A client on wgpu 29 uses `zgx_surface::wgpu29`; one on another wgpu (Horizon is on 30) copies `crates/zgx-surface/src/wgpu29.rs` (about 200 lines with docs, half of it what a client needs) and changes `wgpu` to its own. The wgpu-hal calls it makes are `Device::as_hal`, `Adapter::as_hal`, `vulkan::Device::texture_from_raw` with `TextureMemory::Dedicated`, `enabled_device_extensions` and `Device::create_texture_from_hal`; checked against wgpu 29 here (wgpu 30's wasn't at hand), so a client on 30 checks each still has that shape and that its ash matches zgx-surface's 0.38 (or passes raw handles: `vk::Handle::as_raw`).

```rust
use zgx_surface::{Client, HostMessage, Offer, BufferKind, wgpu29::{self, wgpu}};

let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
let mut offers: Vec<Offer> = wgpu29::vulkan_offer(&adapters).into_iter().collect();
offers.extend(wgpu29::iosurface_offer(&adapters));         // macOS
offers.push(Offer::Shm);
let mut client = Client::connect(offers)?;                 // ZGX_SURFACE_FD (and _MACH)
let adapter = match (client.welcome().kind, client.welcome().device) {
    (BufferKind::Vulkan | BufferKind::IoSurface, Some(ids)) => wgpu29::adapter_for(adapters, &ids).unwrap(),
    _ => todo!("any adapter, and the shm path"),
};
let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
let mut textures = Vec::new();
loop {
    while let Some(event) = client.poll() {
        match event {
            HostMessage::Buffers(generation) => {
                textures = generation.buffers.into_iter()
                    .map(|b| unsafe { wgpu29::import(&device, b) }.unwrap())
                    .collect();
            }
            HostMessage::Key(key) => { /* key.code, key.down */ }
            HostMessage::Button { button, down, x, y } => { /* a press: maybe client.lock_pointer(true) */ }
            HostMessage::PointerMotion { dx, dy } => { /* look about */ }
            HostMessage::Visible(false) => { /* stop drawing */ }
            HostMessage::Close => return Ok(()),
            _ => {}
        }
    }
    if textures.is_empty() || client.free_buffers() < 2 {
        client.wait(None);                                  // a release or an event
        continue;
    }
    let index = client.acquire().unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    draw(&mut encoder, &textures[index as usize]);         // a render pass into it
    let done = queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::Wait { submission_index: Some(done), timeout: None })?;
    client.present(index)?;
}
```

A winit client wakes its loop with `client.set_waker(move || proxy.wake_up())` and polls the client there instead of waiting. Its own window stays unmapped (or is never made). Without a matching Vulkan device it offers `Shm`, draws into a texture of its own and writes it into the buffer's mapping (`wgpu29::Readback`, then `Client::present`).

## Who may start what

- **Rust:** `App::spawn` starts what it is given.
- **Luau:** `ui.app(id, { command, args, cwd, env, on_exit })` ([reference](reference/native.md#apps)). A plugin needs the `apps` grant (`Capability::Apps`). The host decides what runs: `ScriptHost::vet_apps(|launch| …)` is given the whole spec asked for (`AppLaunch { plugin, id, asked: AppSpec { command, args, cwd, env }, running }`) and returns the `AppSpec` to start, started exactly (a path it resolved, pinned arguments, a filtered environment), or an error, which is the script's. Without a hook, what is asked starts. The app's own VM also asks its guard (`GuardKind::App`, named by the spec as JSON, `{"command", "args"?, "cwd"?, "env"?}`).
- Each time a script reaches a running session again (a reload, a lookup, a `ui.surface` given a handle) it is asked again (`running = true`); a refusal kills the program, forgets the session (`App::revoke`) and makes every handle to it raise, its element drawing nothing and forwarding nothing.
- A script reaches only the sessions it started: a plugin's ids are its own, apart from the app's and every other plugin's. Removing a plugin (`set_plugins` without it) ends its programs.

## Measured (Linux)

November 2026, Hyprland on an RTX 4090 (and a 3090, which the client correctly skipped by UUID), NVIDIA 615.78, release builds, the spinner client at 1920 × 1096 device pixels, 10 s each, CPU as a share of one core:

| | host | client | client frames a second |
| --- | --- | --- | --- |
| Vulkan, 60 fps (`--fps 60`) | 2.1–2.8% | 0.6–0.9% | 59.8 |
| shm, 60 fps | 13.8–13.9% | 3.7–4.3% | 59.8 |
| Vulkan, as the 100 Hz display shows them | 2.8% | 1.2% | 100 |
| shm, as the display shows them | 23.2% | 9.0% | 100 |
| Vulkan, 60 fps at 2880 × 1643 | 2.6% | 0.7% | 59.8 |
| shm, 60 fps at 2880 × 1643 | 27.6% | 15.4% | 59.8 |

The Vulkan path's CPU doesn't grow with the frame's size: no CPU touches its pixels. The host's whole frame (zgx's own UI and the surface's draw) costs about 0.35–0.47 ms of CPU at 60 fps. Shared memory costs about 2 ms of host CPU a frame at 1080p (the upload), and grows with the pixels.

Latency, with `ZGX_SURFACE_LOG=1` (the host's share: from the client's present to the host drawing the frame): when the host is otherwise idle it draws at once, 0.03 ms median (p95 0.04–0.08 ms); paced by a 100 Hz display it waits for the next refresh, 9.0 ms median (p95 9.3 ms). The compositor then shows it a refresh or so later: client to screen about 10–20 ms at 100 Hz, as a frame of zgx's own takes.

A client on another wgpu takes IOSurfaces with `zgx_surface::metal::texture(raw_device, surface)` (objc2-metal 0.3, wgpu-hal's), wrapped with its wgpu-hal's `metal::Device::texture_from_raw` and `create_texture_from_hal::<Metal>`; one without wgpu or objc2 uses `Surface::as_ptr` (the `IOSurfaceRef`) itself.

## For the Windows backend

macOS's backend fills the seam this section described (`iosurface_generation` beside `vulkan_generation`, `choose` by its Metal device, an `IoSurface` arm in `wgpu29::import`, the Mach channel beside the socket). What's there for Windows: the protocol (`BufferKind::Dxgi`, `Offer::Dxgi { adapters }`, `BufferDesc::Dxgi`), the Unix transport, the Windows transport (named pipes `<name>.down` and `<name>.up` named in `ZGX_SURFACE_PIPE`, handles duplicated into the client and their values following their frame; compile-checked for `x86_64-pc-windows-gnu`, never run), shared memory on both (`shm`: a file mapping on Windows), and the client (`Client`). What each needs:

1. **The kind's buffers on the host:** in `crates/zgx-surface/src/host/painter.rs`, `size_generation` matches the kind; add `dxgi_generation` beside `vulkan_generation` and `iosurface_generation`, returning `Buffers` (the host's `wgpu::Texture`s by index) and the `Generation` to send.
   - Windows: a `D3D12` committed resource with `D3D12_HEAP_FLAG_SHARED` (or a D3D11 texture with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE`), shared as an NT handle (`CreateSharedHandle`), which the transport duplicates into the client. The client opens it (`OpenSharedHandle`, or Vulkan's `VK_KHR_external_memory_win32` via wgpu-hal's `texture_from_d3d11_shared_handle`) on the adapter whose LUID the host sends.
2. **The choice:** `choose` in `painter.rs` picks the kind from the client's offers and what the device can share (`wgpu29::can_share` and `device_ids_of` are Vulkan's, `can_share_iosurface` and `iosurface_ids_of` Metal's; add an adapter LUID's).
3. **The host's process handling on Windows:** `host::kill` is a no-op there (it should `TerminateProcess` the child's handle) and the exit waiter uses `Child::wait`; `App::spawn` already hands the child's handle to the transport.
4. **The client's import:** a `wgpu29::import` arm for `BufferDesc::Dxgi`.
5. **Sync:** the client waits for its GPU before presenting, as on Linux and macOS; a shared fence (`ID3D12Fence` shared; `MTLSharedEvent` on macOS, whose handle travels as IOSurfaces' do) is the later improvement.

The rest (rings, resize generations, input, pointer lock, the Luau API, permissions) is the same everywhere and needs nothing.
