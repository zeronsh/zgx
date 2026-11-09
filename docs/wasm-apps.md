# WebAssembly apps: a plugin's component in the host's process

A WebAssembly app is a component (`.wasm`, built for `wasm32-wasip2`) that a zgx host runs **inside its own process**, sandboxed, on a thread of its own, shown as an ordinary element with all its input: keys, typed text, the pointer, raw motion under a pointer lock, buttons, the wheel, focus, visibility, size and the display's refresh rate. It draws with **WebGPU on the host's own device**, into a texture the element samples as it is: no copy, no second process, no second GPU context. A plugin ships one `.wasm` that runs on every OS the host does, near native speed, and that can neither crash nor hang the host.

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
| frames | Vulkan images shared across processes (Linux), else shared memory | a texture of the host's own wgpu device, every platform |
| reaches | whatever the process may | nothing but its element and the GPU, unless granted |
| ships as | a binary per OS and architecture | one `.wasm` |
| crashes | its process | its session: a trap, an exit or a call running too long |
| grant | `Capability::Apps` | `Capability::Wasm` (or `Apps`) |

Scripts treat both alike: `ui.app` returns the same `App` (`status`, `title`, `error`, `exit_code`, `close`, `on_exit`; `kind` says which), and `ui.surface { app = … }` shows either.

## The runtime, and why

- **Wasmtime 48**, the newest whose MSRV (1.95) is the workspace's, with the Component Model and WASI 0.2 (`wasm32-wasip2`, what stable Rust builds). One engine for the process (Cranelift); a store per session; each component compiled on its session's thread, so the UI thread never waits for a guest.
- **The GPU is `zgx:app/gpu`, ours**, not `wasi:webgpu`. The wasi-gfx runtime (`wasi-webgpu-wasmtime` 0.5, November 2026) was evaluated: it targets the same wgpu (29), but it makes its own `wgpu_core::Global` and its own adapter and device, and wgpu 29 has no public way to hand it the renderer's (the core global behind a `wgpu::Device` is private), so its frames would be another device's, copied through the CPU or shared with platform interop per backend; `wasi:webgpu` 0.3.0-rc.2 is built on Component Model async (`async func`, streams, futures), which needs Wasmtime's unstable async component support and an async guest; no Rust guest can use `wgpu` over it today (wgpu has no `wasi:webgpu` backend upstream), and its host still unwraps on guest input in places. `zgx:app/gpu` keeps WebGPU's names and rules (a subset, WGSL shaders), so a later move to `wasi:webgpu` is a renaming, not a redesign, once it settles and can wrap a given device.
- **Threads:** a guest runs on its own thread. The UI thread sends it its input, sizes and frame requests over a channel (input beyond 1024 unread events is dropped; sizes and frames never are), and reads what it says (title, cursor, pointer lock, frames) as notes, woken by the guest. Each frame is asked at a display refresh while the element shows and animates (`GpuView::animate`), only once the last was drawn: a slow guest drops frames, never queues them. What it presents shows at the next refresh.
- **Frames:** `gpu.current-texture` hands the guest one of up to four `Bgra8Unorm` textures (viewable as `Bgra8UnormSrgb` where the device can) at the element's size in device pixels, made on the renderer's device. As `frame` returns, that texture is presented; the painter binds it and draws it over the box (one triangle), and gives the one it replaced back to the guest. Guest and renderer submit to the same queue, so wgpu orders the guest's drawing into a texture after the frames that read it: no fence, no wait. `host.present-pixels` takes RGBA8 from the CPU instead, uploaded as the box draws (one copy).

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
| `gpu-reset` | the host's device changed: every GPU object made before is dead; make them again in the next frame |

- **`host`**: `lock-pointer(bool)` (granted within a second of a press or key, as the web's; Escape always lets go), `set-cursor(cursor)`, `set-title(string)` (`app.title`), `log(level, string)`, `set-animating(bool)`, `request-frame()`, `present-pixels(list<u8>, width, height) -> result`.
- **`gpu`**: resources `buffer`, `texture`, `texture-view`, `sampler`, `shader-module` (WGSL), `bind-group-layout`, `pipeline-layout`, `bind-group`, `render-pipeline`, `compute-pipeline` (each with `get-bind-group-layout`), `command-encoder` (render and compute passes, copies between buffers and textures, `clear-buffer`, `finish`), `render-pass` (pipeline, bind groups with dynamic offsets, vertex and index buffers, viewport, scissor, blend constant, stencil reference, draws direct and indirect), `compute-pass` (dispatch direct and indirect), `command-buffer`; functions `create-*`, `write-buffer`, `write-texture`, `submit`, `read-buffer` (a `map-read` buffer, waiting up to 2 s), `current-texture`, `preferred-format`, `ready`, `get-adapter-info`, `get-limits` (the device's and the session's caps). What can fail returns `result<_, string>` with wgpu's validation message; what can't (a pass's commands) reports at `finish` or `submit`, or in the session's log. Not there yet: render bundles, query sets and timestamps, stencil operations, pipeline-overridable constants, `create-*-async`.

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
| GPU | 1 GiB of buffers and textures, 16384 objects, WGSL ≤ 4 MiB, reads ≤ 64 MiB, `present-pixels` ≤ 8192² | `gpu_bytes`, `gpu_objects` |
| stack | 2 MiB of wasm stack; deeper traps | — |

Standard output and error, `host.log` and GPU errors the guest left uncaught are its log (`WasmApp::log()`, the last 256 lines; `ZGX_WASM_LOG=1` prints them). A session ends alone, its status `failed` with why, the app untouched:

- a trap (a Rust panic: the message is its log's `panicked at` line, `` `event` trapped: wasm `unreachable` instruction executed (it panicked: …)``), a stack overflow, an out-of-bounds access;
- a call running past its time (`` `frame` ran longer than 5.0 s: interrupted``);
- memory past its limit (`` `event` trapped: forcing trap when growing memory to … bytes``);
- a panic of the host's while serving it (caught on its thread; wgpu is given only what has been checked where it would panic, such as a buffer's range).

An exit (`wasi:cli/exit`) is `exited` with its code: WASI 0.2 carries only success or failure, so 0 or 1. Closing a session (`app:close()`, a revoked grant, the plugin removed, the last handle dropped) interrupts a call in progress. The ticking thread sleeps while no guest runs: idle sessions cost nothing.

What the sandbox doesn't cover: the guest shares the host's GPU (a pathological shader can stall the GPU until the driver resets it, which zgx sees as a lost device), and it runs in the host's address space, protected by Wasmtime's isolation (guard pages, Spectre mitigations on bounds checks) rather than a process boundary. A host built with `panic = "abort"` loses the host-panic guarantee: keep `unwind`.

## Who may run what

- **Rust:** `WasmApp::spawn` runs what it is given.
- **Luau:** `ui.app(id, { wasm, memory, timeout, on_exit })` ([reference](reference/native.md#apps)). `wasm` is a path from the script's folder; a plugin's must be a file inside its own folder (links followed, `..` refused). A plugin needs the `wasm` grant (`Capability::Wasm`; `Capability::Apps` allows it too, being more). The host's hook `ScriptHost::vet_wasm(|launch| …)` is given `WasmLaunch { plugin, id, asked: WasmSpec { module, memory_limit, timeout, gpu_bytes, gpu_objects, data_dir, wall_clock }, running }` and returns the `WasmSpec` to run (lower or higher limits, a data folder, the time of day) or an error, which is the script's; without a hook, what is asked runs with nothing granted. The app's own VM asks its guard too (`GuardKind::Wasm`, named by the module's resolved path). Both are asked again each time a script reaches a running session; refused, it is closed and its handles raise.

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

`cargo build --release --target wasm32-wasip2` makes the component; put it in the plugin's folder and name it in `ui.app`. Any language whose toolchain makes a WASI 0.2 component against `wit/app.wit` works the same (`wit-bindgen` for C, `componentize-py`, `jco` for JavaScript). The guests in `crates/zgx-wasm/guest`: `cube` (a lit 3D scene with MSAA and depth: drag to orbit, right-click to look with the pointer locked, wheel and arrows zoom, Space, R, C and 1-4 change it, typing builds a wall), `pixels` (CPU-drawn, idle until its input changes), and `misbehave` (traps, loops, hoards memory, exits and tries files on purpose, for the tests). Built, they are 75-125 KB.

## Verified

`cargo test -p zgx-script --features wasm --test state wasm_apps::` (the guests prebuilt in `guest/built`):

- input after a real draw and a resize: the pixels guest's frame drawn by a headless renderer, the element resized, then a press, a key with its text and the wheel heard (the external surfaces' bug of input dying at the first resize was missed by tests that never drew);
- the 3D guest's frames drawn on the renderer's device, turning (skipped without a GPU);
- a trap, a loop (interrupted at 0.3 s) and memory past 32 MiB each fail their session alone, another session still hearing its input; an exit's code; no file reachable;
- reloads keep a session, `close` ends it; grants, a plugin's folder, the hook and the guard.

On Linux (Hyprland, NVIDIA, Vulkan), `examples/wasm_apps` draws the 3D guest at the display's rate (100 fps at 100 Hz, 144 at 144 Hz, as the window moved).
