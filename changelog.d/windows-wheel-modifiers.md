---
section: fixed
---
- **Shift with the wheel over a window without focus** (zgx-desktop on Windows): a mouse message's Shift and Control are now read from the message, so a window behind another scrolls across with Shift and the wheel, and Shift- or Control-clicks there are seen. They were read from the keyboard's state, which only the focused window's thread has. A winit-only fix: apps on crates.io's winit keep the old behaviour.
