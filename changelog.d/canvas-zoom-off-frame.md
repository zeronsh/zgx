---
section: performance
---
- **A rasterized canvas zooms without rasterizing on the frame** (zgx-gpu): a `ui.canvas` with a `view` whose drawing rasterizes rasterized its box again on each zoom step, on the frame. In a window, a zoom step now shows the raster of the zoom before, scaled by the GPU, while the new zoom's is made on a thread of its own; the window is woken and draws it once made. One is made at a time per canvas, for the latest zoom. A design board's zoom step went from 70 to 82 ms on the frame (p95) to under 1 ms, sharp again 65 to 85 ms later. Offscreen renderers still rasterize on the frame.
