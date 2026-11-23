//! Where a window asked for at a position is made on macOS, for every
//! arrangement of screens, as data: the arithmetic of the zgx patch to
//! winit (`vendor/winit/src/platform_impl/macos/placement.rs`), which has no
//! AppKit in it, runs here on every platform. `window_position` checks the
//! same with real screens.
#![allow(dead_code)]

#[path = "../../../vendor/winit/src/platform_impl/macos/placement.rs"]
mod placement;
