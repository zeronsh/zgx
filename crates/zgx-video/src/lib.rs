//! Videos written as zgx interfaces: an ordinary view or Luau script played on
//! a virtual clock ([`zgx::clock::VirtualClock`]), a frame at a time, as fast
//! as the machine draws, and the same way each time.
pub mod analysis;
pub mod audio;
mod blend;
pub mod captions;
pub mod clip;
pub mod compose;
pub mod encode;
pub mod film;
pub mod player;
pub mod render;
pub mod script;
pub mod stage;
pub mod studio;
pub use encode::Format;
pub use film::{Film, Ticks, Timing};
pub use player::{Event, Player};
pub use render::{Progress, RenderOptions, Report, Still, render, render_still};
pub use script::{AudioClip, Composition, ScriptStage, Track, compositions};
pub use stage::{Stage, StageOptions};
