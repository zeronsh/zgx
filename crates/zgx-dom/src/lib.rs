//! zgx views as browser DOM, for WebAssembly.
//!
//! A `zgx::compose::View` mounts straight into real DOM elements: the
//! browser lays it out (zgx's flexbox and grid defaults are CSS's), measures
//! and shapes its text, scrolls, selects and handles IME, and zgx's reactive
//! graph, which already knows what changed, writes only the nodes that read
//! it. There is no virtual DOM and no diffing:
//!
//! - each dynamic part (bound text, a reactive style, an attribute, an
//!   editor's value) is one zgx effect owning one node;
//! - its writes go into one operation buffer that a small JS side applies
//!   once per microtask, so a frame crosses from wasm to JS once;
//! - events are delegated from the root, one listener per kind;
//! - static styles become shared CSS classes, and hover, pressed and focus
//!   styles become `:hover`, `:active` and `:focus-visible` rules, which run
//!   no wasm at all;
//! - keyed lists move the fewest nodes (a longest increasing subsequence),
//!   with fast paths for appending and clearing; each row is a block that
//!   owns its effects and nodes and goes as one.
//!
//! See the crate's README for what maps, what doesn't, and the benchmark.
#![cfg(target_arch = "wasm32")]

#[cfg(feature = "audio")]
mod audio;
mod canvas;
#[cfg(feature = "luau")]
mod fetch;
mod idle;
mod js;
mod keyed;
mod mount;
mod ops;
mod platform;
mod sheet;
mod tasks;
mod template;

#[doc(hidden)]
pub use idle::{trim_armed, trims};
pub use mount::{Mounted, flush, live_nodes, mount, mount_in_body};
#[doc(hidden)]
pub use tasks::{frame_asked, timer_armed};

/// Sound for a Luau app's `audio`: zgx-audio pulled by an audio worklet
/// (`audio`).
#[cfg(feature = "audio")]
pub use audio::{__audio_drain_and_wake, __audio_state, install_audio, install_audio_at};
/// The page's `fetch` as a Luau app's transport (`ui.fetch`): relative
/// URLs read against the app's base (`luau`).
#[cfg(feature = "luau")]
pub use fetch::{__fetches_pending, install_fetcher, install_fetcher_at};

/// Whether a frame is asked for a drag held near a scroll view's edge (for
/// tests: none outside the band). Not API.
#[doc(hidden)]
pub fn drag_scroll_pending() -> bool {
    js::zgx_drag_scroll_pending()
}

/// zgx-script, whose Luau apps zgx-dom mounts (`luau`): a host's `view()`
/// mounts as any view does.
#[cfg(feature = "luau")]
pub use zgx_script as script;

/// Whether repeated rows are stamped from templates (on by default). For
/// tests that compare the stamped DOM with the DOM written in full; not API.
#[doc(hidden)]
pub fn __set_templates(on: bool) {
    template::set_enabled(on);
}
