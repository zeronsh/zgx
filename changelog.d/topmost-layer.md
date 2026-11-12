---
section: added
---
- **A topmost layer above the app's popup windows** (zgx, zgx-script, zgx-desktop): `View::topmost()` (Luau `ui.topmost { open?, … }`, or `topmost = true` on `ui.layer`) is a `layer` (no focus, no backdrop, the pointer passing through but for the controls in it) that macOS draws in a borderless, transparent panel of its own over the app's menus, popovers, selects, tooltips and a drag's preview, covering its window's content with its root at 0, 0, and following the window as it moves, resizes, zooms and changes Space. Of two, the later is above. It is a popup root only while open and painting something, so it costs nothing idle; a press on a control in it closes no menu or popover. Where the host has no such windows it is the window's top layer (`top_layer`).
