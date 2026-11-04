---
section: performance
---
- **Dragging and scrolling keep up on a 120 Hz Mac** (zgx-desktop): a window's frames for the next 100 ms after the pointer or the wheel moves hand over as they are drawn, not at a fixed point before a refresh, which cost a frame of about 8 ms a whole refresh. Streams and animations with no hand on them keep that point. `zgx inspect switch refresh_grid off` still turns it off for all frames.
