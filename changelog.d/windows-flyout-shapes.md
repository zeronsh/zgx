---
section: changed
---
- **Menus, popovers and tooltips take Windows 11's shapes on Windows** (zgx-desktop): as WinUI 3 draws its flyouts, a menu's panel is rounded 8 with 2 above and below its items, each item's highlight inset 4 by 2 and rounded 4, items padded 11 by 4; a popover is rounded 8; a tooltip is padded 9 by 6 in 12-point text (`zgx::theme::windows_look`, set as the system's tokens, which an app's theme overrides). Colours stay the theme's. macOS and Linux are unchanged.
