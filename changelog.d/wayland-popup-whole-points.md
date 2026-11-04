---
section: fixed
---
- **A menu opened between pixels stays sharp on Wayland** (zgx-gpu, zgx-desktop): a popup at a place that isn't a whole pixel (a context menu at the pointer) drew a buffer a pixel wider or taller than its surface, all of it resampled to fit, its text blurred. A popup now shows on the whole points round its box, its buffer exactly its surface, and the pointer lands on what it shows there. `zgx_gpu::plane_size` says what a plane's surface should measure.
