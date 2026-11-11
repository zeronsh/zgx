---
section: added
---
- **`WGPU_FORCE_FALLBACK_ADAPTER=1` draws on WARP even beside a GPU** (zgx-gpu, Windows): the renderer's device, Direct3D 12's and Direct3D 11's, is on Windows' software adapter, as on a machine without a GPU, such as CI's runners. A machine with a GPU can then run the tests CI runs on WARP and bless software goldens. Unset, the adapter is chosen as before: the first GPU DXGI ranks for low power. On Linux it takes a software Vulkan driver; macOS has no software Metal adapter, so there the renderer fails to start with it set.
