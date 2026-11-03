---
section: added
---
- **A window's glass blurs the desktop behind it on Linux** (zgx-desktop): a window with `glass` asks the Wayland compositor to blur what is behind it, through `ext-background-effect-v1` (Hyprland, KWin) or KWin's own blur protocol where that is all there is (winit's `with_blur`, which now prefers the standard protocol). It was only transparent before, so on KWin, which blurs only what asks, it was see-through. How strong the blur is stays the compositor's (Hyprland: `decoration:blur`, and only with blur enabled there).
