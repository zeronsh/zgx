//! Wayland protocol implementation boilerplate.

pub mod cursor;
// zgx patch: blur behind a window through `ext-background-effect-v1`.
pub mod ext_background_effect;
pub mod kwin_blur;
pub mod wp_fractional_scaling;
pub mod wp_viewporter;
pub mod xdg_activation;
