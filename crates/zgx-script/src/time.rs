//! The clock's types: std's, and in the browser (wasm32) `web-time`'s,
//! which read `performance.now()` where std's panic, as zgx's own do.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::{Instant, SystemTime};
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::{Instant, SystemTime};
