---
section: performance
---
- **A canvas that pans and zooms draws its text into its raster** (zgx, zgx-gpu): a `ui.canvas` with a `view` kept every text as a text node over its raster, so each zoom step laid out and drew every word again, at the new size, on the frame, and text could show stale after a zoom. Its text is now drawn into its raster with its shapes, in their layers, cut by its clips and turned by its transforms (as `ui.render_canvas` draws it), and pans and zooms as pixels with the rest (#2100, #2101). On a design board with 323 texts, a zoom step went from 63 to 247 ms to 12 to 21 ms, and a frame of a panel resize from 29 to 8 ms (debug build).
