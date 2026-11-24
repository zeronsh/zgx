---
section: fixed
---
- **A window asked for on a screen of another scale opens there on macOS** (zgx-desktop, through winit): a window's physical position and size (`WindowBounds`, a restored frame, `display`) were read at the scale of the key window's screen. A window asked for on a 1x screen beside a 2x one opened at the wrong place or size, or on the other screen. Both are now read at the scale of the screen that holds the window's top left.
