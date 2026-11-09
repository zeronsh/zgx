//! WebAssembly apps for zgx: a plugin's component run inside the host's
//! process, sandboxed, drawing with WebGPU on the host's own device, shown
//! as an element with all its input: the in-process counterpart of
//! zgx-surface's external surfaces (another process).
//!
//! ```no_run
//! # fn demo(runtime: &zgx::reactive::Runtime) -> zgx::compose::View {
//! use zgx::compose::prelude::*;
//! use zgx_wasm::{WasmApp, WasmOptions, surface};
//! let app = WasmApp::spawn(runtime, WasmOptions::new("cube.wasm"));
//! surface(&app).w(640.).h(400.).rounded(16.)
//! # }
//! ```
//!
//! The component is built for `wasm32-wasip2` against the `zgx:app/app`
//! world (`wit/app.wit`; `examples/wasm/zgx-app` is a Rust guest's SDK).
//! It runs on a thread of its own, never the UI's: its frames are asked at
//! the display's refresh while it shows, and what it draws shows at the
//! next. It reaches nothing but its element and the GPU: no files (unless
//! the host grants a folder), network, environment, time of day or other
//! program, its memory and every call's time capped. A trap, an exit, a
//! call running too long ends its session alone. docs/wasm-apps.md says
//! more.
mod app;
mod bundles;
mod cache;
mod device;
mod gpu;
mod loops;
mod painter;
mod pipelines;
mod runtime;
mod shared;

pub use app::{WasmApp, WasmOptions, WasmStatus, surface};
pub use cache::{DEFAULT_LIMIT as DEFAULT_CACHE_LIMIT, set_cache};

/// The longest side `present-pixels` takes.
pub(crate) const MAX_PIXELS_SIDE: u32 = 8192;
