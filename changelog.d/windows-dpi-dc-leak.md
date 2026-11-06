---
section: fixed
---
- **A window on Windows no longer runs out of GDI handles** (vendored winit): reading a window's DPI took a device context and never gave it back, and the occlusion check reads it each second, so after an hour or two `GetDC` failed and the app panicked with "`GetDC` returned null". The context is now taken only where it's read, and released.
