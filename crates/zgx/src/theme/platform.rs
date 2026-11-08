//! Platforms' colours for zgx's widgets: existing tokens a desktop host
//! sets as the system's (`Themes::set_system_tokens`), which an app's theme
//! overrides. (Their shapes are constants zgx is built with: `OverlayLook`,
//! the scrollbar's `Look`.)

use super::{TokenId, TokenInput, tok};

/// Windows 11's: keyboard focus in the text's colour, as WinUI's focus
/// visual's outer ring (`FocusStrokeColorOuter`: white on dark, near-black
/// on light; drawn outside the control on Windows, `focus_outside`); its
/// scrollbar's thumb the strong fill (`ControlStrongFillColorDefault`, the
/// text at about half), over a track the surface's colour (its in-app
/// acrylic's) as it widens.
pub fn windows_look() -> Vec<(TokenId, TokenInput)> {
    let value = |value: &str| TokenInput::parse(value).expect("a token value");
    vec![
        (tok::FOCUS_RING.0, value("{text.primary}")),
        (tok::SCROLLBAR_THUMB.0, value("alpha({text.primary}, 0.5)")),
        (tok::SCROLLBAR_TRACK.0, value("{surface.raised}")),
    ]
}
