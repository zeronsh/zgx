---
section: added
---
- **A window can animate itself into full screen and out, live** (zgx-desktop, zgx-script, macOS): `ui.window.live_full_screen`, `"live_full_screen"` in `zgx.json` and `WindowOptions::live_full_screen` (`WindowHandle::set_live_full_screen`), off by default. AppKit's standard animation moves a snapshot of the window, and what it draws meanwhile isn't seen; with this on, the live window goes to the screen's frame and back, its content drawing and resizing on the screen through the transition, so a title bar's slide timed to `ui.window.full_screen_transition` is seen as it runs.
