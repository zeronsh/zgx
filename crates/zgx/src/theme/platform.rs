//! Platforms' looks for zgx's widgets: tokens a desktop host sets as the
//! system's (`Themes::set_system_tokens`), which an app's theme overrides.

use super::{TokenId, TokenInput, tok};

/// Windows 11's shapes for menus, popovers and tooltips, as WinUI 3 draws
/// its MenuFlyout, FlyoutPresenter and ToolTip (microsoft-ui-xaml's
/// `CommonStyles`), in the theme's own colours: a menu's panel rounded as
/// overlays are (`OverlayCornerRadius`, 8) with 2 above and below its items
/// and none across, each item's highlight inset 4 across and 2 down
/// (`MenuFlyoutItemMargin`) and rounded as controls are
/// (`ControlCornerRadius`, 4), padded 11 across and 4 down (the narrow
/// padding a mouse or keyboard opens it with); a popover rounded 8; a
/// tooltip padded 9 across, 6 above and 8 below (`ToolTipBorderPadding`) in 12-point text
/// (`ToolTipContentThemeFontSize`). No border on menus and popovers: in
/// windows of their own, the default, DWM draws their rim. Keyboard focus in
/// the text's colour, as its focus visual's outer ring (drawn outside the
/// control on Windows, `focus_outside`).
pub fn windows_look() -> Vec<(TokenId, TokenInput)> {
    let value = |value: &str| TokenInput::parse(value).expect("a token value");
    vec![
        (tok::MENU_RADIUS.0, value("8")),
        (tok::MENU_PADDING_X.0, value("0")),
        (tok::MENU_PADDING_Y.0, value("2")),
        (tok::MENU_ITEM_PADDING_X.0, value("11")),
        (tok::MENU_ITEM_PADDING_Y.0, value("4")),
        (tok::MENU_ITEM_INSET_X.0, value("4")),
        (tok::MENU_ITEM_INSET_Y.0, value("2")),
        (tok::MENU_ITEM_RADIUS.0, value("4")),
        (tok::POPOVER_RADIUS.0, value("8")),
        (tok::TOOLTIP_PADDING_X.0, value("9")),
        (tok::TOOLTIP_PADDING_Y.0, value("6")),
        (tok::TOOLTIP_PADDING_BOTTOM.0, value("8")),
        (tok::TOOLTIP_FONT_SIZE.0, value("12")),
        // Its focus visual's outer ring in the text's colour
        // (`FocusStrokeColorOuter`: white on dark, near-black on light);
        // its inner ring is `focus.ring.inner`'s default.
        (tok::FOCUS_RING.0, value("{text.primary}")),
    ]
}
