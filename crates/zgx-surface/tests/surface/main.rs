//! External surfaces: the protocol, a session over a socket pair, and Vulkan
//! memory shared between two devices. One test binary for the crate (see
//! AGENTS.md): a test file goes beside the others, listed here.
#[path = "../protocol.rs"]
mod protocol;
#[path = "../shm_session.rs"]
mod shm_session;
#[path = "../vulkan_share.rs"]
mod vulkan_share;
