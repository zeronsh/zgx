---
section: fixed
---
- **A browser's animation frames never go back in time** (zgx-dom): a frame's time is at least the last one's. Before, a coarse clock, or a refresh interval shortened at once for a faster display, could put a frame's time before the previous frame's. A browser can still give two frames the same timestamp, so a frame can come with no time passed.
