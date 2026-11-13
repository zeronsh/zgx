---
section: fixed
---
- **Shaders kept on disk are never read back cut short** (zgx-gpu, Windows): threads compiling the same shader at once (concurrent tests) wrote one temporary file per process, so one could rename it into place while another was still writing it, and a reader then passed part of the bytecode to Direct3D 12, where WARP crashed reading past its end. Each writer now writes a file of its own, and a cached shader is used only as a whole DXBC container, at the size its header gives. This applies to both the pipeline cache (wgpu-hal's) and the app-shader cache (`fxc_child`).
