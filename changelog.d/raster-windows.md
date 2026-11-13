---
section: added
---
- **A window can draw on the software raster where the only GPU is a software one** (zgx-desktop, zgx-gpu): with `ZGX_RENDERER=raster`, an opaque 1x window draws with the raster and shows its frames through softbuffer, copying only what changed. With `ZGX_RENDERER=auto` it does so where the machine has no hardware adapter, found without opening a device (`zgx_gpu::software_only`: no render node on Linux, only software adapters in DXGI on Windows), or where the GPU renderer's adapter turns out to be a software one. An idle app's resident memory fell from 140 MB on lavapipe to 42 MB, and a frame costs a fraction of the CPU (0.2–0.5 ms where lavapipe and WARP took 1.3–9.6). A window whose scale leaves 1x moves to the GPU. The GPU stays the default.
