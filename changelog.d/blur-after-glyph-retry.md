---
section: fixed
---
- **A blur or mask shown first in a frame that grows the glyph atlas is drawn** (zgx-gpu): the frame is drawn again with the atlas larger, and the effect slots the failed try had planned were taken as drawn, so the node showed whatever its slot held before (another paint's pixels, often a square) until its content changed.
