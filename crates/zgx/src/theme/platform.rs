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
/// tooltip padded 9 by 6 (`ToolTipBorderPadding`, 9,6,9,8) in 12-point text
/// (`ToolTipContentThemeFontSize`). No border on menus and popovers: in
/// windows of their own, the default, DWM draws their rim.
pub fn windows_look() -> Vec<(TokenId, TokenInput)> {
    let number = |value: &str| TokenInput::parse(value).expect("a number");
    vec![
        (tok::MENU_RADIUS.0, number("8")),
        (tok::MENU_PADDING_X.0, number("0")),
        (tok::MENU_PADDING_Y.0, number("2")),
        (tok::MENU_ITEM_PADDING_X.0, number("11")),
        (tok::MENU_ITEM_PADDING_Y.0, number("4")),
        (tok::MENU_ITEM_INSET_X.0, number("4")),
        (tok::MENU_ITEM_INSET_Y.0, number("2")),
        (tok::MENU_ITEM_RADIUS.0, number("4")),
        (tok::POPOVER_RADIUS.0, number("8")),
        (tok::TOOLTIP_PADDING_X.0, number("9")),
        (tok::TOOLTIP_PADDING_Y.0, number("6")),
        (tok::TOOLTIP_FONT_SIZE.0, number("12")),
    ]
}
