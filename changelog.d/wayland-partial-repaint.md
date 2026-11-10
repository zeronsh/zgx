---
section: performance
---
- **On Wayland a frame repaints only what changed** (zgx-desktop): every frame repainted the whole window. A frame the compositor granted was always drawn whole, and damage that arrived while the window waited for it was promoted to the whole window. Now that damage is kept and painted with the granted frame's own; each present still copies the whole retained frame, so nothing relied on the swap chain's old contents. On a headless sway, a switches test changing about 8% of a 1280x800 window and a hover over two 24-point cells had repainted all of it on every frame.
