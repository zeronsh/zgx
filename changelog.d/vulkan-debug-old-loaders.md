---
section: fixed
---
- **Debug builds no longer crash on Vulkan loaders before 1.4.345** (zgx-gpu): those loaders' debug-utils calls race across threads (Vulkan-Loader#1866, Ubuntu 26.04's 1.4.341), and wgpu's debug layer makes them every frame. On such a loader a debug build leaves that layer off and says so once on stderr; `ZGX_VULKAN_DEBUG=1` turns it back on. Release builds never used it.
