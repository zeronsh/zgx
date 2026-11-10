---
section: performance
---
- **A photo shrunk to the size it's drawn takes a third of the time** (zgx, zgx-gpu): `resample::shrink` takes its area mean down each target row's few source rows and then across, a fixed few taps each way with each axis's weights summing to one, instead of over the whole area with a division a texel. It's within 1/255 of what it made before. A 1600 by 1200 JPEG drawn 1083 by 812 shrinks in 2.9 ms, not 8.3 (thread CPU time, M5 Max), so a fast scroll through photos shows each one sooner. The decoding thread premultiplies sixteen pixels at a time, passing over opaque blocks (every JPEG's), as the renderer does: the two share `zgx::image::premultiply`.
