---
section: fixed
---
- **A material's shadow is cast by the GPU data bound now, whichever it is** (zgx-3d, `shaders` and `compute` features): what decides that a shadow cast by a material's GPU data is cast anew now also covers a buffer or texture swapped for another by `set_param`, not only the same one written, and looks only at the shaders the last frame drew as casters.
