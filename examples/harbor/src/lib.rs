//! A harbor at dawn: the reference demo of zgx-3d's primitives, built on
//! the public API of the zgx crates alone (design-3d-aaa.md, section 10).
//!
//! Everything in it is made by code here: the quay, the container yard, the
//! gantry cranes, the ship, the water and every texture. Nothing is
//! downloaded and nothing is copied. Water, weather and terrain are the
//! demo's, written against the engine's primitives; what the primitives
//! can't express is listed in `GAPS.md`, not worked around.
//!
//! - [`scene`] builds the world, deterministically from a seed.
//! - [`path`] is the camera's fly-through, a function of frame time.
//! - [`settings`] are the panel's values, each bound to the world.
//! - [`hud`] is the zgx interface over the view.
//! - [`weather`] is the rain, on particle emitters.
//! - [`perf`] reads the process's counters, for `harbor_bench`.
pub mod hud;
pub mod path;
pub mod perf;
pub mod rng;
pub mod scene;
pub mod settings;
pub mod shapes;
pub mod sky;
pub mod textures;
pub mod weather;
