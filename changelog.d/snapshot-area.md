---
section: performance
---
- **A picture of what a window showed scales only when it must** (zgx-desktop, macOS): a snapshot that fits its width is converted a pixel at a time instead of box-filtered, about 4× faster (a 1151×817 pt area at 2×: 12.9 ms to 2.7 ms). A picture of part of a node reads and scales only that part, so a dragged row's preview out of a long note takes about 0.1 ms, not 13 ms.
