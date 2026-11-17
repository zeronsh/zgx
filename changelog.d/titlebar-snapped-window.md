---
section: fixed
---
- **A window snapped by a window manager keeps its title bar's buttons where they are** (zgx-desktop, macOS): a window resized by a frame set from outside (Raycast, Rectangle and others set it through Accessibility, its position then its size), or by a drag, reported the inset title bar's buttons (`WindowInfo::controls`, `ui.window.controls`) as far down the window as its height grew, and the title bar's area as tall as that, until the window next moved or resized: winit reports the resize from the content view, before AppKit moves the title bar to the new top. They're measured from the title bar's own container now, so a title bar laid out around the traffic lights stays beside them.
