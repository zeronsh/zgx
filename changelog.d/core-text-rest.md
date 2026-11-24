---
section: performance
---
- **A window at rest gives back the glyphs Core Text drew** (zgx-gpu, zgx-desktop, macOS): the fonts zgx made to draw faces swash can't (macOS 26's PingFang) were kept for good, and Core Text keeps each font's glyphs while it lives. After a page of Chinese text read through at three sizes that was some 30 MB of heap, 40 MB of footprint, at rest. The idle trim now lets them go, with the GPU renderer and the software raster alike; the next glyph drawn makes its font again.
