---
section: fixed
---
- **The software raster draws rich text where and as the GPU renderer does** (zgx-desktop, zgx-gpu): it laid rich text out in a font system of its own, its glyphs kept their fractions of a pixel, and none was thickened for its run's colour as macOS's font smoothing thickens it, so a window falling back to the raster drew rich text unlike the GPU (4,900 pixels off, by up to 207 of 255, in the parity test). It now shares the renderer's fonts, layout and placement, as plain text did, each run keyed for its colour.
