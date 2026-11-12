---
section: performance
---
- **Frames paced by the timer no longer skip or repeat a refresh** (zgx-desktop, macOS): the next frame's deadline is the refresh after the last one delivered, not the first after the moment it's set. With the timer's grid a little off the display's, a frame ending just past a refresh's start used to skip it, and a timer waking just before one delivered the last refresh again: on a long list scrolled by the wheel, 412 skips and 363 repeats in 43,000 refreshes became 64 and 1, and refreshes missed for it about 70% fewer. More than half a refresh behind, a frame still skips to the next rather than catching up.
