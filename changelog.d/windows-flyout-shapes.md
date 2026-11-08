---
section: changed
---
- **Menus, popovers and tooltips take Windows 11's shapes on Windows** (zgx): as WinUI 3 draws its flyouts, a menu's panel is rounded 8 with 2 above and below its items, each item's highlight inset 4 by 2 and rounded 4, items padded 11 by 4; a popover is rounded 8; a tooltip is padded 9 across, 6 above and 8 below, in 12-point text. Colours stay the theme's. They are zgx's constants for Windows, not theme tokens: a view's own styles restyle them. macOS and Linux are unchanged.
