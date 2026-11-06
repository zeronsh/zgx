---
section: fixed
---
- **A double click on a `window_drag` region zooms the window live** (zgx-desktop, macOS): the window's last frame was stretched over it for the whole animation, then snapped to the new layout. The zoom ran inside the event that asked for it, so winit held every size AppKit reported until the animation ended. It now runs once that event has been handled, as a zoom from the Window menu does.
