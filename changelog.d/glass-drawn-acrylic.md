---
section: changed
---
- **Drawn glass on Windows is Windows' Acrylic**: a `glass` box on Windows, which has no system glass for a box, drew macOS's glass. It now draws Acrylic as WinUI's `AcrylicBrush` makes it: the backdrop blurred with a sigma of 30, a luminosity blend and a colour blend with the theme's tint, and its noise at 0.02 (the base acrylic for regular glass, the default for clear, light or dark). With Transparency effects off it is the brush's fallback colour, as Windows' own goes solid. Where the window shows the system's own blur behind the box (a frosted popup, a glass window), it lays its tint alone over that, so the blur isn't darkened twice. Direct2D's own effects running the same recipe over the same backdrops read within a level of it.
