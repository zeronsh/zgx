---
section: performance
---
- **A window on the software raster lets its canvases' rasters go at rest** (zgx-desktop): the raster kept the pixels of every canvas and icon it had drawn, though its own pixels hold what they drew; a GPU renderer lets them go once uploaded. At rest they go too (a terminal's rows are canvases: 1.8 MB at 900 by 600), and a repaint over one rasterizes it again.
