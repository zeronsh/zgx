//! The clock's types: std's, and on the web (wasm32) the browser's
//! `performance.now()` and `Date.now()` through `web-time`, where std's
//! panic. The same types as std's everywhere else.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::{Instant, SystemTime};
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::{Instant, SystemTime};
