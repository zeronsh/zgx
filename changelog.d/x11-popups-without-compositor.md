---
section: fixed
---
- **Popups look right on X11 with no compositor, and under picom** (zgx-desktop): with no compositing manager, a popup's window kept the margin its shadow draws in and its corners' clear pixels, all of them black, and translucent colours were drawn over black: a menu showed as a black box, its light theme's text hard to read. Its window is now its panel alone, corners cut, and `ui.prefers.reduced_transparency` (`system_preferences`) is true there, so glass turns solid; a compositor started later is heard as a window is activated. Under picom, whose own shadow went round that whole margin, a popup now asks for none (`_COMPTON_SHADOW`).
