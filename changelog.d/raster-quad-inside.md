---
section: performance
---
- **The software raster fills a box's inside without its coverage math** (zgx-desktop): a window drawn on the raster (a machine with no GPU) took a rounded-corner distance and its square root for every pixel of every box it painted, though inside the box, past its corners and its border, that comes to the fill colour. Those pixels are now filled directly (a plain fill when opaque); the corners, edges and border as before, pixel for pixel. A terminal under output, repainting its background each frame, used 17% less CPU (the 200 MB flood in a window on Linux, 2.11 s to 1.75 s) and 20% less for `seq 1 1000000`.
