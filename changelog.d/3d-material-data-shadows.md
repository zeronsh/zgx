---
section: fixed
---
- **A material moved or cut out by GPU data casts its shadow as the data is now** (zgx-3d, `shaders` and `compute` features): a mesh whose `displace` or `opacity` reads a `GpuBuffer` or `GpuTexture` a compute pass or the app writes kept the shadow it first cast. It is cast anew on each frame that changed the data, and kept on frames that didn't.
