---
section: fixed
---
- **An inset title bar leaves no bar over the content in full screen** (zgx-desktop, macOS): AppKit moves a window's toolbar into a window of its own above it in full screen, which the title bar's transparency doesn't reach, so `Titlebar::Inset`'s empty toolbar showed as an opaque 40-point bar over the content's top. It is now hidden as the window goes into full screen (the green button or `set_fullscreen`) and shown again once it is out, where it centres the traffic lights.
