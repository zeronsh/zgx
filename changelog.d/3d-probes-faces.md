---
section: added
---
- **Irradiance probes light a 3D world on devices without ray queries** (zgx-3d, `gi` feature): each probe draws the scene into six small cube faces about itself and reads them along its rays, where the device has compute but no ray queries (Direct3D 12 compiling with FXC, macOS before 15, Vulkan without ray queries). `World::probes_status()` says `Traced`, `Rasterised`, or `Unsupported` on a device without compute (GL).
