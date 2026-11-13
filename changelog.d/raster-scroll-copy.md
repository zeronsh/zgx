---
section: performance
---
- **The software raster backend moves a scroll's pixels rather than repainting its view** (zgx-desktop, zgx-gpu): `Raster::render` copies what scrolled by whole pixels within its view and repaints only the strip scrolled in, and what the move carried of anything over or around the view, with the GPU renderer's own rules (`zgx_gpu::scroll_copy_repaints`). A 1280 by 800 list scrolled at 0.17 ms a frame where it took 13.3.
