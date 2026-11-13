---
section: fixed
---
- With Direct3D 11 (`ZGX_D3D11`), `GpuRenderer::debug_present_offscreen` gives the frame as it would be presented instead of an error, so tests of scroll copies and presentation run under Direct3D 11 too.
