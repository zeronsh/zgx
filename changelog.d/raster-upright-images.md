---
section: performance
---
- **The software raster draws an image without a transform a row at a time** (zgx-desktop): each pixel of an image (a canvas's drawing too: a terminal's rows are canvases) was mapped back through the image's transform, divided into its texel and clamped, though with no transform and no dither the texel's column depends on the pixel's column alone and its row on its row. Those are now worked out once each and the pixels blended along the row, as before pixel for pixel; and a clear or opaque colour is blended without the sums it comes to.
