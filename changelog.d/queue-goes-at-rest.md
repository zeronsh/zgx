---
section: performance
---
- **A window at rest gives Metal's queue back** (zgx-gpu, wgpu-hal): the driver keeps a queue's memory for as long as it lives, a pool of command buffers' resources (as many as were ever in flight at once) and render scratch: 174 MB on an M5 Max, whatever is drawn. As a window goes idle its queue goes, with all it was given finished, and the next frame makes another. On an M5 Max, about 200 MB less within seconds of the last frame; the first frame after takes no longer than on a queue that has sat a few seconds.
