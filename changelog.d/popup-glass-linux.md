---
section: fixed
---
- **A popup's glass blurs on Linux, and past its window's edge everywhere it's drawn this way** (zgx-gpu, zgx-desktop): on X11 and Wayland a menu, popover or tooltip with `backdrop_blur` blurred nothing, as on Windows before. Both now blur what the window last drew under them, and on all three a popup reaching past its window carries the window's edge on, so a dropdown's glass is opaque throughout instead of fading to sharp near the edge.
