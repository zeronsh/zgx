---
section: changed
---
- **Windows starts without compiling the quad shader** (zgx-gpu): on Direct3D 12 with FXC (the default), draw.wgsl's stages come compiled ahead of time and are handed to wgpu as bytecode, the same FXC would make at launch, so a window's first frame skips naga's translation, the shader cache's lookup and, cold, FXC itself. Elsewhere, and in debug builds, they compile as before.
