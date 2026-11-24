---
section: fixed
---
- **A window asked for on another screen opens there on macOS** (zgx-desktop, through winit): a window given a position (`WindowBounds`, a restored frame) on a display other than the one holding the key window opened on the key window's display instead, its height mirrored, as AppKit reads a new window's rect relative to the key window's screen unless it is given one. It is now made on the screen its top left is on, in that screen's coordinates.
