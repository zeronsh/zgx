//! External surfaces: another process draws frames, with its own GPU work,
//! and a zgx app shows them as an element, forwarding its input back.
//!
//! The host (zgx, `host` feature) starts the program with one end of a
//! connection inherited (`ZGX_SURFACE_FD` on Unix). The program, the
//! client, says what it can take (`Hello`), the host says how buffers are
//! shared (`Welcome`), then sends rings of buffers at the element's size
//! (`Buffers`, a generation per size). The client draws into a free one,
//! finishes its GPU work and presents it; the host shows the newest
//! presented and gives each back (`Release`) once it no longer reads it.
//!
//! - `vulkan` buffers (Linux): Vulkan images the host allocates on its GPU,
//!   their memory exported as opaque file descriptors, imported by a
//!   client on the same device. No pixel is copied by a CPU.
//! - `shm` buffers (everywhere): shared memory the client writes pixels
//!   into and the host uploads.
//! - `iosurface` (macOS) and `dxgi` (Windows): named, not yet implemented
//!   (see `docs/external-surfaces.md` for what they need).
//!
//! This crate's core has no wgpu: a client on any wgpu version (or none)
//! uses the protocol (`protocol`), its connection (`transport`, `Client`),
//! shared memory (`shm`) and, with `vulkan`, ash helpers to import images.
//! `wgpu29` turns buffers into wgpu 29 textures; a client on another wgpu
//! copies that module.
pub mod client;
#[cfg(feature = "host")]
pub mod host;
pub mod protocol;
pub mod ring;
pub mod shm;
pub mod transport;
#[cfg(feature = "vulkan")]
pub mod vulkan;
#[cfg(feature = "wgpu29")]
pub mod wgpu29;

pub use client::Client;
pub use protocol::{
    Buffer, BufferDesc, BufferKind, Button, ClientMessage, Cursor, DeviceIds, Generation,
    HostMessage, KeyEvent, Modifiers, Offer, PROTOCOL_VERSION, PixelFormat, VulkanImage, WheelUnit,
};
pub use transport::in_surface;

/// A handle a buffer's memory travels as: a file descriptor on Unix.
#[cfg(unix)]
pub type Handle = std::os::fd::OwnedFd;
/// A handle a buffer's memory travels as: a `HANDLE` on Windows.
#[cfg(windows)]
pub type Handle = std::os::windows::io::OwnedHandle;
