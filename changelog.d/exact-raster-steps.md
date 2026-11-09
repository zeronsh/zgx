---
section: fixed
---
- **A layer magnified 2 or 4 times reads its own texels exactly**: its raster scale was √2 squared, a hair under 2, so a layer shader's read of a texel landed just off its centre, and some GPUs (an RTX 4090 and WARP on Direct3D 12) sampled a little of the next one: 253 where 255 was drawn. Whole steps of two are now exact.
