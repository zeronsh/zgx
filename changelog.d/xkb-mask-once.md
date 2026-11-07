---
section: performance
---
- **A pointer move on X11 doesn't work out the keyboard's modifiers again** (zgx-desktop on Linux): every move carries them, and the keyboard state was updated from them each time; it is now updated only when they change. About 400 to 1,600 instructions fewer a move under cachegrind (1–5%). A winit-only change: apps on crates.io's winit keep the old cost.
