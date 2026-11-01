---
section: added
---
- **A window's largest size** (zgx-desktop, zgx-script, zgx-run): `WindowOptions::max_size` and `WindowHandle::set_max_inner_size`, beside `min_size`; zgx.json's `max_width` and `max_height`; and `ui.window { min_width, min_height, max_width, max_height }`. The same minimum and maximum make a window of fixed size that stays resizable to the system, so window managers such as Raycast can move it on macOS, which they refuse for one that isn't resizable.
