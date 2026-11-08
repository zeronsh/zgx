---
section: performance
---
- **A video's frames come back already RGBA** (zgx-gpu, zgx-video): `GpuRenderer::readback_into` copies the BGRA target to an RGBA texture on the GPU (the present blit, a load and a store) before reading it, so the read is a copy of rows with nothing swapped a pixel at a time. Frames are byte-identical; readback went from 7.9 M to 5.3 M instructions a 1080p frame, the showcase's render from 31.0 G to 25.7 G.
