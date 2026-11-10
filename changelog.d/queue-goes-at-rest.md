---
section: performance
---
- **A window at rest a while gives Metal's queue back** (zgx-gpu, wgpu-hal, zgx-desktop): the driver keeps a queue's memory for as long as it lives, a pool of command buffers' resources and render scratch, up to 174 MB on an M5 Max. A window at rest 10 s past its idle trim lets its queue go, and the next frame makes another. A queue idle that long starts as slowly as a new one, so the frame after pays nothing more.
