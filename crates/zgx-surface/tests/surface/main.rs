//! External surfaces: the protocol, a session over a socket pair, Vulkan
//! memory shared between two devices and Direct3D 12 textures between two
//! processes. One test binary for the crate (see
//! AGENTS.md): a test file goes beside the others, listed here.
#[path = "../dxgi_share.rs"]
mod dxgi_share;
#[path = "../protocol.rs"]
mod protocol;
#[path = "../shm_session.rs"]
mod shm_session;
#[path = "../vulkan_share.rs"]
mod vulkan_share;
