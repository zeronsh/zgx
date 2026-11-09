# WebAssembly apps: a plugin's component in the host's process

A WebAssembly app is a component (`.wasm`, built for `wasm32-wasip2`) that a zgx host runs **inside its own process**, sandboxed, on a thread of its own, shown as an ordinary element with all its input: keys, typed text, the pointer, raw motion under a pointer lock, buttons, the wheel, focus, visibility, size and the display's refresh rate. It draws with **WebGPU on a device of its own** on the adapter zgx draws with, into textures zgx's device shares with it (Vulkan memory, IOSurfaces, Direct3D 12 handles): no copy, no second process, and what goes wrong on its GPU (a lost device, a reset, a shader that won't end) is its session's alone. A plugin ships one `.wasm` that runs on every OS the host does, near native speed, and that can neither crash nor hang the host.

```luau
local game = ui.app("game", { wasm = "game.wasm" })
return ui.surface { app = game, w = "100%", h = 600, rounded = 16 }
```

```rust
use zgx::compose::prelude::*;
use zgx_wasm::{WasmApp, WasmOptions, surface};

let app = WasmApp::spawn(&cx.ui.runtime, WasmOptions::new("game.wasm"));
cx.render(surface(&app).w_full().h(600.).rounded(16.).overflow_hidden());
```

`crates/zgx-script/examples/wasm_apps` shows a lit 3D scene and a CPU-drawn app side by side (build the guests first with `crates/zgx-wasm/guest/build.sh`):

```sh
crates/zgx-wasm/guest/build.sh                       # needs `rustup target add wasm32-wasip2`
cargo run -p zgx-run -- crates/zgx-script/examples/wasm_apps
```

## Against external surfaces

| | [External surfaces](external-surfaces.md) (`command`) | WebAssembly apps (`wasm`) |
| --- | --- | --- |
| runs | another process, any language, any GPU API | this process, a thread of its own, Wasmtime |
| frames | Vulkan images, IOSurfaces or Direct3D 12 textures shared across processes, else shared memory | the same, shared in process with a device of its own; else read back |
| reaches | whatever the process may | nothing but its element and the GPU, unless granted |
| ships as | a binary per OS and architecture | one `.wasm` |
| crashes | its process | its session: a trap, an exit or a call running too long |
| grant | `Capability::Apps` | `Capability::Wasm` (or `Apps`) |

Scripts treat both alike: `ui.app` returns the same `App` (`status`, `title`, `error`, `exit_code`, `close`, `on_exit`; `kind` says which), and `ui.surface { app = … }` shows either.

## The runtime, and why

- **Wasmtime 48**, the newest whose MSRV (1.95) is the workspace's, with the Component Model and WASI 0.2 (`wasm32-wasip2`, what stable Rust builds). One engine for the process (Cranelift); a store per session; each component compiled on its session's thread, so the UI thread never waits for a guest, and kept compiled on disk ([the cache](#the-compile-cache)).
- **The GPU is `zgx:app/gpu`, ours**, not `wasi:webgpu`. The wasi-gfx runtime (`wasi-webgpu-wasmtime` 0.5, November 2026) was evaluated: it targets the same wgpu (29), but it makes its own `wgpu_core::Global` and its own adapter and device, and wgpu 29 has no public way to hand it the renderer's (the core global behind a `wgpu::Device` is private), so its frames would be another device's, copied through the CPU or shared with platform interop per backend; `wasi:webgpu` 0.3.0-rc.2 is built on Component Model async (`async func`, streams, futures), which needs Wasmtime's unstable async component support and an async guest; no Rust guest can use `wgpu` over it today (wgpu has no `wasi:webgpu` backend upstream), and its host still unwraps on guest input in places. `zgx:app/gpu` keeps WebGPU's names and rules (WGSL shaders), so a later move to `wasi:webgpu` is a renaming, not a redesign, once it settles.
- **Threads:** a guest runs on its own thread. The UI thread sends it its input, sizes and frame requests over a channel (input beyond 1024 unread events is dropped; sizes and frames never are), and reads what it says (title, cursor, pointer lock, frames) as notes, woken by the guest. Each frame is asked at a display refresh while the element shows and animates (`GpuView::animate`), only once the last was drawn: a slow guest drops frames, never queues them. What it presents shows at the next refresh.
- **Frames:** zgx's painter makes a ring of three `Bgra8Unorm` textures (viewable as `Bgra8UnormSrgb`) at the element's size in device pixels on its own device, shares them as an external surface's buffers are (Vulkan memory as an opaque file descriptor on Linux, IOSurfaces on macOS, Direct3D 12 shared handles on Windows) and the guest's device imports them; a new size gets a new ring once it has held 120 ms. `gpu.current-texture` hands the guest a free one; as `frame` returns, the guest's thread waits for its device's work (the watchdog) and presents it; the painter draws it over the box (one triangle) and gives the one it replaced back once zgx's GPU has read it (`on_submitted_work_done`). Where nothing can be shared (GL, a driver without the extension, `ZGX_WASM_READBACK=1`) the guest draws into textures of its own, read back and uploaded. `host.present-pixels` takes RGBA8 from the CPU instead, uploaded as the box draws (one copy).

## The WIT world

`crates/zgx-wasm/wit/app.wit` is the definition: package `zgx:app@0.1.0`, world `app`.

```wit
world app {
    import host;     // the element
    import gpu;      // WebGPU on the host's device
    export init: func(info: surface-info) -> result<_, string>;
    export frame: func(time: f64);
    export event: func(event: input-event);
}
```

- **`init(info)`**, once, first: `surface-info { width, height, scale, refresh }`, the element's size in device pixels, device pixels a point, the display's rate (0 until known). An `err` fails the session with its text.
- **`frame(time)`**: draw. `time` is seconds since the session began, by the display's clock (animate by it and an app turns as fast at 60 Hz as at 144). Called every refresh while the element shows, unless the app called `host.set-animating(false)`; then once after each `host.request-frame()`, and after a resize.
- **`event(e)`**, `input-event`, the external surfaces' vocabulary:

| variant | |
| --- | --- |
| `key(key-event)` | `code` as the DOM's `KeyboardEvent.code` (`"KeyW"`, `"Space"`, `"ShiftLeft"`), `down`, `repeat`, `modifiers` (flags: shift, control, alt, meta). Modifiers alone arrive as their keys |
| `text(string)` | what a key typed, or an input method's commit |
| `pointer-move(point)` | device pixels from the element's top left |
| `pointer-motion(motion)` | raw `dx`, `dy` while the pointer is locked |
| `button(button-event)` | `left`, `right`, `middle`, `back`, `forward`, `other(n)`; `down`, `x`, `y` |
| `wheel(wheel-event)` | `dx`, `dy` (positive scrolls down), `unit` (`pixels`) |
| `pointer-leave`, `pointer-lock(bool)`, `focus(bool)`, `visible(bool)` | |
| `resize(surface-info)`, `refresh(f32)` | a new size or scale; the display's rate as known and as the window moves |
| `gpu-reset` | the app's device was made anew (zgx moved to another adapter): every GPU object made before is dead; make them again in the next frame |
| `pipeline-ready(u64)` | a pipeline made with `create-*-pipeline-async` is ready: its future's `id` |

- **`host`**: `lock-pointer(bool)` (granted within a second of a press or key, as the web's; Escape always lets go), `set-cursor(cursor)`, `set-title(string)` (`app.title`), `log(level, string)`, `set-animating(bool)`, `request-frame()`, `present-pixels(list<u8>, width, height) -> result`.
- **`gpu`**: resources `buffer`, `texture`, `texture-view`, `sampler`, `shader-module` (WGSL), `bind-group-layout`, `pipeline-layout`, `bind-group`, `render-pipeline`, `compute-pipeline` (each with `get-bind-group-layout`), `command-encoder` (render and compute passes, copies between buffers and textures, `clear-buffer`, `finish`), `render-pass` (pipeline, bind groups with dynamic offsets, vertex and index buffers, viewport, scissor, blend constant, stencil reference, draws direct and indirect), `compute-pass` (dispatch direct and indirect), `command-buffer`; functions `create-*`, `write-buffer`, `write-texture`, `submit`, `read-buffer` (a `map-read` buffer, waiting up to 2 s), `current-texture`, `preferred-format`, `ready`, `get-adapter-info`, `get-limits` (the device's and the session's caps). Also:
  - **render bundles**: `create-render-bundle-encoder` (colour formats, depth-stencil, sample count 1 or 4, checked before wgpu), the pass's commands recorded once, `finish` → `render-bundle`, run by `render-pass.execute-bundles`;
  - **queries**: `create-query-set { kind: occlusion | timestamp, count ≤ 4096 }`, `render-pass-descriptor.occlusion-query-set` with `begin-occlusion-query`/`end-occlusion-query`, `timestamp-writes` on render and compute passes, `command-encoder.resolve-query-set` into a `query-resolve` buffer, `get-timestamp-period`; timestamps where the adapter has them (`supports-timestamps`), a plain error where not;
  - **stencil**: `depth-stencil-state.stencil` (both faces' compare, fail, depth-fail and pass operations, read and write masks), `render-pass-depth-attachment.stencil-clear`/`stencil-store`, `set-stencil-reference`;
  - **overridable constants**: `constants` (`list<tuple<string, f64>>`) on vertex, fragment and compute stages, for WGSL's `override`;
  - **async pipelines**: `create-render-pipeline-async` and `create-compute-pipeline-async` return a future, made on a thread of their own (16 at once at most); `pipeline-ready(id)` comes as an event, and `take` gives the pipeline (or why not).

  What can fail returns `result<_, string>` with wgpu's validation message; what can't (a pass's commands) reports at `finish` or `submit`, or in the session's log.

GPU objects are made in `frame` (the device is always there in one; before the first, `ready()` may be false: a host drawing without a GPU never has one).

## The sandbox

Each session's store grants nothing but what the host says:

| | default | granted by the host's hook |
| --- | --- | --- |
| files | none: no preopened directory (WASI 0.2 reaches files only through one) | a folder of its own as `/data` (`WasmSpec::data_dir`) |
| network | none: TCP, UDP and name lookups refused | — |
| environment, arguments, stdin | empty, closed | — |
| time of day | the Unix epoch | `WasmSpec::wall_clock` |
| monotonic clock | whole milliseconds (enough to pace; too coarse to time the host's caches by) | — |
| randomness | the host's secure random, as WASI gives it | — |
| linear memory | 1 GiB; a growth past it traps | `memory_limit` |
| each call's time | 5 s for `init`, a `frame`, an `event`; then interrupted (epoch interruption, checked every 10 ms) | `timeout` |
| GPU memory | 1 GiB of buffers, textures and recorded bundle commands, 16384 objects, WGSL ≤ 4 MiB, reads ≤ 64 MiB, `present-pixels` ≤ 8192², 65536 commands a bundle | `gpu_bytes`, `gpu_objects` |
| GPU work | a device of its own; 10 000 draws and 2²⁴ workgroups a submission (counted by the encoder recording them, summed over what a submission runs; indirect dispatch refused under a cap below the device's largest); one submission on the GPU at a time; 2¹⁶ loop iterations a shader invocation; 2 s of GPU work for a call, then the session fails | `max_draws`, `max_workgroups` (`work_caps`), `shader_loop_budget`, `gpu_watchdog` |
| stack | 2 MiB of wasm stack; deeper traps | — |

Standard output and error, `host.log` and GPU errors the guest left uncaught are its log (`WasmApp::log()`, the last 256 lines; `ZGX_WASM_LOG=1` prints them). A session ends alone, its status `failed` with why, the app untouched:

- a trap (a Rust panic: the message is its log's `panicked at` line, `` `event` trapped: wasm `unreachable` instruction executed (it panicked: …)``), a stack overflow, an out-of-bounds access;
- a call running past its time (`` `frame` ran longer than 5.0 s: interrupted``);
- memory past its limit (`` `event` trapped: forcing trap when growing memory to … bytes``);
- a panic of the host's while serving it (caught on its thread; wgpu is given only what has been checked where it would panic, such as a buffer's range).

An exit (`wasi:cli/exit`) is `exited` with its code: WASI 0.2 carries only success or failure, so 0 or 1. Closing a session (`app:close()`, a revoked grant, the plugin removed, the last handle dropped) interrupts a call in progress. The ticking thread sleeps while no guest runs: idle sessions cost nothing.

What the sandbox doesn't cover: the guest runs in the host's address space, protected by Wasmtime's isolation (guard pages, Spectre mitigations on bounds checks) rather than a process boundary, and it shares the GPU's time ([below](#the-gpu-a-device-of-its-own)). A host built with `panic = "abort"` loses the host-panic guarantee (wgpu panics on some input, caught where the guest reaches it): keep `unwind`.

## The GPU: a device of its own

Each session opens a wgpu device of its own on zgx's adapter, matched by its Vulkan device and driver UUIDs, Metal registry id or Direct3D LUID (else vendor, device and name), with the adapter's limits and timestamp queries where it has them. What happens to it is the session's:

- **a device lost or reset** (a driver's recovery, `VK_ERROR_DEVICE_LOST`, `DXGI_ERROR_DEVICE_REMOVED`): the session fails, `its GPU device was lost: …`; zgx's device, another, goes on;
- **too long a call's GPU work**: past the watchdog (2 s by default, every submission of a call together, measured from the call's start) the session fails, `its GPU work for a call ran longer than 2.0 s: stopped`;
- **its device let go**: only once the GPU is done with it, on a thread of its own (wgpu's queue waits a few seconds as it drops and then panics, and destroying what the GPU still uses is undefined), the store's objects with it; work that never ends keeps it until the driver loses the device. Once the process exits, a device is left to the driver's teardown rather than destroyed beside it (on NVIDIA's driver that crashed: `vkDestroyDevice` against `exit`'s handlers).

A separate device doesn't make the GPU preempt, though. Measured on an RTX 4090 (NVIDIA 615.78, Linux, Vulkan): one fragment shader looping two billion times a pixel held zgx's own frames 4.3 s from another device; split into a hundred draws of about 24 ms each, a submission of them still held zgx's frame for its whole length (4.9 s). The driver turns to another device's work only between submissions, never inside one invocation. So:

- **every loop of a guest's shader is bounded**: its WGSL is parsed with naga (as wgpu would), and each loop's body first counts against a private per-invocation budget (2¹⁶ iterations, its loops together) and breaks once past it. A ray marcher's hundreds of steps and a blur's thousands run as written; a stall's billions are cut. Loops needing uniform control flow (a barrier, `textureSample`, `dpdx`, a subgroup operation, inside or in a function they call) are left as written, with a line in the log;
- **its submissions run one at a time**, each waiting for the last under the watchdog, and each may record so many draws and workgroups;
- measured so (a 2¹⁶ budget, 100 draws a submission, ten submissions of the two-billion loop): zgx's slowest frame was 157 ms, one guest submission; the session failed at 0.8 s with a 0.5 s watchdog; its device was let go 0.12 s after.

The watchdog is the wall clock's: another's heavy work delays a session's too, so keep it well above a frame (the default 2 s). If the driver resets the whole GPU (a Windows TDR after its 2 s timeout, an NVIDIA Xid that takes the adapter down), every device on it is lost: zgx's renderer makes its own anew (zgx-gpu's lost-device path), every session's device is lost and the session fails; a script starts it again with `ui.app`. Not tested here: Windows TDRs, macOS's command buffer timeouts, AMD and Intel scheduling.

## The compile cache

A start loads a module compiled before rather than compiling it: compiled components are kept in the host's cache folder (the platform's cache folder's `zgx/wasm`: `$XDG_CACHE_HOME`, `~/.cache`, `~/Library/Caches`, `%LOCALAPPDATA%`), named by the module's SHA-256 and the engine's compatibility hash (Wasmtime's version, its configuration, the target), written whole (a temporary file renamed over), touched as used, the least recently used removed past 512 MiB. `zgx_wasm::set_cache(dir, limit)` (`zgx_script::set_wasm_cache`) moves, resizes or turns it off (`None`). Compiled code is loaded only from that folder, which only the host writes, never from bytes a plugin gives; a broken entry is compiled anew. The 3D guest (139 KB) compiles in 22.7 ms and loads in 0.3 ms; the session's log says which.

## Who may run what

- **Rust:** `WasmApp::spawn` runs what it is given.
- **Luau:** `ui.app(id, { wasm, memory, timeout, on_exit })` ([reference](reference/native.md#apps)). `wasm` is a path from the script's folder; a plugin's must be a file inside its own folder (links followed, `..` refused). A plugin needs the `wasm` grant (`Capability::Wasm`; `Capability::Apps` allows it too, being more). The host's hook `ScriptHost::vet_wasm(|launch| …)` is given `WasmLaunch { plugin, id, asked: WasmSpec { module, memory_limit, timeout, gpu_bytes, gpu_objects, gpu_watchdog, max_draws, max_workgroups, shader_loop_budget, data_dir, wall_clock }, running }` and returns the `WasmSpec` to run (lower or higher limits, a data folder, the time of day) or an error, which is the script's; without a hook, what is asked runs with nothing granted. The app's own VM asks its guard too (`GuardKind::Wasm`, named by the module's resolved path). Both are asked again each time a script reaches a running session; refused, it is closed and its handles raise.

## Shipping one

A Rust guest uses `crates/zgx-wasm/guest/zgx-app` (bindings by `wit-bindgen` and an `App` trait):

```rust
struct Hello { size: (u32, u32) }
impl zgx_app::App for Hello {
    fn init(info: zgx_app::SurfaceInfo) -> Result<Self, String> {
        zgx_app::host::set_animating(false);
        Ok(Hello { size: (info.width, info.height) })
    }
    fn frame(&mut self, _time: f64) {
        let (w, h) = self.size;
        zgx_app::host::present_pixels(&vec![255; (w * h * 4) as usize], w, h).unwrap();
    }
    fn event(&mut self, event: zgx_app::InputEvent) {
        if let zgx_app::InputEvent::Resize(info) = event {
            self.size = (info.width, info.height);
        }
    }
}
zgx_app::app!(Hello);
```

```toml
[lib]
crate-type = ["cdylib"]
[dependencies]
zgx-app = { path = "…/crates/zgx-wasm/guest/zgx-app" }
```

`cargo build --release --target wasm32-wasip2` makes the component; put it in the plugin's folder and name it in `ui.app`. Any language whose toolchain makes a WASI 0.2 component against `wit/app.wit` works the same (`wit-bindgen` for C, `componentize-py`, `jco` for JavaScript). The guests in `crates/zgx-wasm/guest`: `cube` (a lit 3D scene with MSAA, mirrored in its floor through the stencil, its cubes a render bundle, its reflections' pipeline made asynchronously, its pass timed by timestamp queries: drag to orbit, right-click to look with the pointer locked, wheel and arrows zoom, Space, R, C and 1-4 change it, typing builds a wall), `pixels` (CPU-drawn, idle until its input changes), `features` (each of constants, stencil, bundles, occlusion and timestamp queries and async pipelines tried and read back) and `misbehave` (traps, loops, hoards memory, exits, tries files, stalls its GPU and probes the GPU caps on purpose, for the tests). Built, they are 75-145 KB.

## Verified

`cargo test -p zgx-script --features wasm --test state wasm_apps::` (the guests prebuilt in `guest/built`):

- input after a real draw and a resize: the pixels guest's frame drawn by a headless renderer, the element resized, then a press, a key with its text and the wheel heard (the external surfaces' bug of input dying at the first resize was missed by tests that never drew);
- the 3D guest's frames drawn on the renderer's device, turning (skipped without a GPU);
- a trap, a loop (interrupted at 0.3 s) and memory past 32 MiB each fail their session alone, another session still hearing its input; an exit's code; no file reachable;
- reloads keep a session, `close` ends it; grants, a plugin's folder, the hook and the guard;
- each new GPU feature on the guest's own device (`the_gpu_features_each_work`; timestamps 1–2 µs a pass, 16 occlusion samples of a 4 × 4 target);
- seconds of a guest's GPU work: its session fails by the watchdog while zgx keeps drawing (`a_stalling_shader_fails_its_session_and_zgx_keeps_drawing`, slowest frame about 157 ms);
- the GPU caps can't be dodged: an encoder past them can't be finished, encoders submitted together are capped together, indirect dispatch is refused under a cap, bundles keep to their caps and bad bundle descriptors are refused before wgpu;
- `cargo test -p zgx-wasm`: the loop budget's rewriting validates (and leaves loops needing uniformity alone), and a second start loads from the compile cache (LRU eviction, a broken entry compiled anew).

On Linux (Hyprland, NVIDIA, Vulkan), `examples/wasm_apps` draws the 3D guest at the display's rate (100 fps at 100 Hz, 144 at 144 Hz, as the window moved), on its own device, its pass 0.10 ms of GPU time.
