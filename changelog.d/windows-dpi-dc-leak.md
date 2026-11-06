---
section: fixed
---
- **A window on Windows no longer runs out of GDI handles** (this repository's winit: apps from crates.io get upstream's, which has no occlusion check reading it that often): reading a window's DPI took a device context and never gave it back, and the occlusion check reads it twice a second while a window draws and each second while it's covered, so after an hour or two of that `GetDC` failed and the app panicked with "`GetDC` returned null". The context is now taken only where it's read, and released.
