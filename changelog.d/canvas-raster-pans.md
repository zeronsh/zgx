---
section: performance
---
- **A rasterized canvas pans without rasterizing** (zgx-gpu): a `ui.canvas` with a `view` whose drawing rasterizes (curves, clips, gradients, shadows: a design board's) rasterized all of its box again on every pan step. Its raster now moves with the view in whole device pixels, and a pan onto more of the drawing rasterizes only that part, kept beside the rest up to the 32 MiB raster budget; a zoom rasterizes what shows, as before. A 328-item design board at 1.5x went from 84 ms of rasterizing per pan step to 0.01 ms (p95), with an occasional step of a few milliseconds where a pan first shows a part.
