---
section: performance
---
- **A video reads each frame back in about a third of the instructions** (zgx-gpu, zgx-video): `GpuRenderer::readback_into` reads into a buffer the caller keeps, through a staging buffer it keeps too, and copies each row and swaps red and blue in one pass a pixel a word. A render no longer allocates and fills a fresh 8 MB frame and staging buffer for each 1080p frame: the showcase's readback went from 21.5 M to 7.8 M instructions a frame, and its whole render from 68.3 G to 41.3 G, with every frame's bytes the same.
