# zgx patch to wgpu-hal 29.0.4 (Metal backend, three Direct3D 12 additions, one Vulkan)

Vendored from crates.io `wgpu-hal` 29.0.4 (MIT OR Apache-2.0, license files
unchanged) and applied through `[patch.crates-io]` in the workspace
`Cargo.toml`. Only `src/metal/{command,mod,device,adapter}.rs` differ, plus
the new `src/metal/buffer_writes.rs`, and for Direct3D 12
`src/dx12/instance.rs`, `src/auxil/dxgi/factory.rs`,
`src/dx12/shader_compilation.rs`, `src/dx12/mod.rs`, `src/dx12/adapter.rs`
and the new `src/dx12/shader_disk_cache.rs` (see the Direct3D 12 sections),
`Cargo.toml`'s `metal` feature (naga's WGSL in, for single-use shader
modules), and for Vulkan `src/vulkan/{adapter,command,device,mod}.rs` and
`src/vulkan/swapchain/native.rs` (the last section); search for "zgx patch".

## Why

Upstream creates a real `MTLCommandBuffer` for every hal encoder that
wgpu-core opens, including the internal "transit" encoders around each pass
and submission that usually record nothing, and commits a separate command
buffer just to present. A zgx frame with two render passes committed about
seven command buffers. On Apple silicon each one costs a commit, an IOKit
submission, completion callbacks on GCD worker threads and Core Animation
frame-pacing IPC (`FPCAMetalLayerState`), which showed up as kernel time: 2.6%
system CPU for zgx vs 1.4% for GPUI on the same animation
(`comparisons/*/animate`, `zgx-desktop/examples/animation_workload`).

## What

1. **Lazy command buffers.** `begin_encoding` records the label; the Metal
   command buffer is created on first use. Encoders that record nothing end
   with no buffer, and `submit` skips them. The fence signal attaches to the
   last real buffer (or, as upstream, to an internal one when a submission is
   entirely empty), so completion ordering is unchanged.
2. **Present without an extra command buffer.** `presentDrawable:` presents
   from the command buffer's scheduled handler. `submit` now registers that
   handler on its last buffer before commit, and `present` hands the drawable
   to it: whichever happens second presents, without blocking. Queue order
   makes the last submission's scheduling cover all earlier rendering. When
   there is no recorded submission, or for transactional presents, the
   upstream path is used.

3. **Buffer fills and copies without blit encoders.** `clear_buffer` and
   `copy_buffer_to_buffer` run as vertex-only render passes (rasterization
   off, one vertex per 32-bit word; see `buffer_writes.rs`), falling back to
   blits for unaligned ranges, same-buffer copies or pending timer queries.
   The first blit encoder a process uses makes the Apple silicon driver hold
   about 110 MB for roughly eight seconds and some of it for good, and
   wgpu-core issues two buffer blits while creating every device. A small
   zgx window measured 297 MB of graphics memory at launch and 65 MB at rest
   before, 185 MB and 41 MB after. Each pass renders to a 1x1 private
   texture it neither loads nor stores: two attachment-less passes in one
   command buffer (wgpu-core's fill and copy at device creation) made the
   driver keep a second set of six 512 KB regions for the life of the
   process, 3 MB of footprint. Tested by
   `crates/zgx-gpu/tests/buffer_writes.rs`.

4. **Exact waits.** `Device::wait` without a timeout blocks on the command
   buffer (`waitUntilCompleted`) instead of polling its status every
   millisecond, so a renderer waiting for its frame before presenting it
   (zgx's own surfaces, `GpuRenderer::present_layer`) presents as the GPU
   finishes rather than up to a millisecond later.

5. **No per-submission extras nothing reads.** Fences have no
   `MTLSharedEvent` (only `Fence::raw_shared_event` interop read it, and
   signalling it is kernel work in every submission), and the scheduled
   handler that presents drawables is registered only on submissions that
   use a surface texture.

Result: two command buffers per frame (one per render pass), 1.2 ms less
kernel time per 100 frames, and zgx below GPUI in CPU per frame.

## Metal: single-use shader modules let go of their IR

wgpu-core keeps every shader module alive with each render pipeline made
from it, and the Metal backend keeps the module's naga IR (module and
validation info) so it can write MSL as a pipeline is made. zgx compiles each
app shader (a fill, a layer shader, a blend, an effect's pass) into a module
holding the whole quad shader, makes one pipeline from it and drops it, and
yet each pipeline kept about 250 KB of IR. A heavy session in Zeron held
29 MB of it.

`metal::zgx_single_use_next_module(Some(wgsl))` says the next module the
calling thread makes from WGSL is for one render pipeline, with `wgsl` the
source wgpu parses for it (zgx-gpu's `single_use_module` sets it just
before, and takes it back just after). Once a render pipeline is made from
such a module, its source becomes `ShaderModuleSource::Spent` and the IR is
freed; the module keeps the WGSL (about 13 KB for the quad shader). A later
pipeline from a spent module parses that WGSL again for that pipeline, and
the module stays spent. Spending costs nothing more than the drop. The
source sits behind an `RwLock` for this, and the `metal` feature turns on
naga's `wgsl-in`. Modules made otherwise are unchanged, and compute
pipelines never spend a module. zgx-gpu's `single_use_module_tests` and
`device_loss` tests cover a second pipeline and a fill drawn again on a new
device.

## Direct3D 12: one adapter, one device

`Instance::enumerate_adapters`, which wgpu-core's `request_adapter` ranks,
opens a Direct3D 12 device on every DXGI adapter to read its capabilities.
Each driver it loads keeps memory after the device is released: an NVIDIA
machine with an indirect display (a second adapter on the same GPU) kept
22 MiB of private memory for the second device, and zgx's shared context
measured 89.8 MiB private where it is 81.0 MiB with one device.

`dx12::Instance::expose_preferred_adapter(power)` exposes the first adapter,
in DXGI's own order for that power preference (`EnumAdapterByGpuPreference`,
`EnumAdapters1` without DXGI 1.6), that has Direct3D 12, a GPU before
software (WARP) whatever that order, and opens no device on the rest; `auxil::dxgi::factory::enumerate_adapters_by` is the ordered
enumeration. zgx-gpu (`preferred_dx12_adapter`) passes the result to
`wgpu::Instance::create_adapter_from_hal`, and falls back to
`request_adapter` when it finds none or it cannot present to the window.
Tested, for low power and high performance, by
`crates/zgx-gpu/tests/adapter.rs`.

## Direct3D 12: compiled shaders kept between launches

FXC compiles the HLSL naga writes for each pipeline stage when a pipeline
is first made, and wgpu-hal keeps the result for the device alone. A zgx
window makes its pipelines at its first frame: six compiles, about 45 ms of
a 330 ms start to first text on an RTX 4090 machine, every launch.

`dx12::set_shader_cache_dir(dir)` keeps FXC's output in `dir`
(`shader_disk_cache.rs`). A file is named by a hash of everything FXC was
given (its flags, target, entry point and source) and holds that key in
full, which must match byte for byte before its bytecode (a DXBC container)
is used: a hash collision, other flags or another version's HLSL compile
again. Files are written whole and renamed into place, so another process
never reads half of one. DXC's output is kept the same way, its key also
naming the DLL's version and its file's size and time (the DLL is the
app's own, so another one compiles again); a DLL found only by name on the
search path, with no file to read, isn't kept. zgx-gpu sets `%LOCALAPPDATA%\zgx\shaders`
(`keep_compiled_shaders`); a window's shaders take about 52 KB there. With
them kept, its first text came at 278 ms instead of 330 ms (medians of 12
alternating launches), sooner than GPUI Fast's 307 ms, whose shaders are
compiled when it is built. Tested by `crates/zgx-gpu/tests/shader_cache.rs`:
a second device reads the shaders the first compiled (spoilt on disk, its
pipeline fails; restored, it works) and compiles none again.

`dx12::shader_cache_counts()` says what the cache did in the process: the
shaders it read back, those FXC compiled, and those compiled that it could
not keep (which compile again next launch). zgx's first-present startup
mark carries them (`ZGX_PROFILE=1`: `first frame presented (shaders: 6
read, 0 compiled, 0 not kept)`), so a start slowed by compiling says so.
`ZGX_SHADER_LOG=1` prints that line alone, for a probe timing starts
(`ZGX_PROFILE`'s spans add about 25 ms to a start).

## Direct3D 12: WARP's submissions one at a time

On WARP 10.0.26100 (Windows 11 24H2 and 25H2), wgpu devices in one process
that execute command lists at the same time make the driver fault
(`d3d10warp.dll`, an access violation under `ExecuteCommandLists`), or draw
nothing with no error. Direct3D 12 hands every caller the one device of the
adapter, so each wgpu device is a queue of its own on that device. The
fault needs those queues executing together: on 16 threads drawing a
triangle each, a debug build faulted in 38 of 40 runs; with one lock
around `ExecuteCommandLists` and the fence signal after it, or one queue
for every device, in 0 of 20 each. Locking pipeline creation, shader
compiles, resource creation or device creation, or committed resources in
place of placed ones, didn't help. Raw Direct3D 12 drawing from 8 threads
with a queue each did not fault, so something in what wgpu records is also
needed. What that is remains unknown: the lock works around the fault, and
the trigger inside wgpu's recording is still to be found.

The adapter's `Workarounds::serialize_submits` is set for WARP (any version),
and `Queue::submit` holds the process-wide `WARP_SUBMITS` lock across
`ExecuteCommandLists` and `Signal`. A GPU's queues take no lock. zgx-gpu also
shares one wgpu device among its renderers on WARP. Tested by
`crates/zgx-gpu/tests/warp_devices.rs`: sixteen threads, each with a wgpu
device of its own on WARP, drawing and reading back at once.

## Vulkan: a present copies what its swap chain image lacks

zgx presents on Linux by copying its retained frame into the swap chain
image it acquires. Copying all of it every frame was most of what was left
of a frame's CPU under a software rasterizer (a 1280x800 copy for a hover
changing two cells), so zgx-gpu now copies only what that image lacks: the
changes of every frame since the one it holds (`SwapHistory`, buffer age by
image). That rests on the spec alone: presentation "will not affect the
content of the presentable images", and an image reacquired and moved out of
`PRESENT_SRC_KHR` holds "the same as they were prior to transitioning the
image to the present source layout" (`vkQueuePresentKHR`).

- **The barrier out of a presented image.** wgpu-core tracks every surface
  texture it acquires as `UNINITIALIZED`, which `derive_image_layout` makes
  `UNDEFINED`, a layout the driver may discard contents leaving.
  `Texture::presented` (a swap chain image's flag, shared from
  `NativeSwapchain::presented` into each texture acquired for it, set as it
  is presented) makes `transition_textures` leave `PRESENT_SRC_KHR`
  instead, where the image is. An image's first use leaves `UNDEFINED` as
  before. A new swap chain starts with every flag clear.
- **Present regions.** `VK_KHR_incremental_present` is enabled where the
  device has it, and `Surface::set_next_present_regions` (beside
  `set_next_present_time`) names the next present's changed rectangles,
  chained as `VkPresentRegionsKHR`; Mesa's WSI hands them to
  `wl_surface.damage_buffer`, so the compositor redraws them alone. Without
  the extension the regions are dropped; the partial copy doesn't need it.
- **`Buffer::raw_handle`**, for the readback `ZGX_SWAP_CHECK` records.

The copy itself is zgx-gpu's (`copy_lacking`), recorded through
`CommandEncoder::as_hal_mut` in a command buffer of its own: a wgpu copy into
part of a texture it holds uninitialized clears all of it first. Any doubt
copies all of it: an image new or of unknown age, a resize or recreate, a
suboptimal acquire, an expose or the window shown again (`forget_presented`).
`scripts/partial_repaint_smoke.py` reads every partly copied image back under
`ZGX_SWAP_CHECK` (sway and X11, a resize and a hide/show mid-stream) and
finds none differing from its frame.
