---
section: performance
---
- **A list that is being resized builds no rows ahead until its width has stood still** (zgx): while a splitter is dragged or a window resized, the rows in view reflow every frame and rows kept out of view are let go, as before; spare time no longer builds them ahead at each new width (a fresh build and layout a frame), but waits until the width has been still for 100 ms.
