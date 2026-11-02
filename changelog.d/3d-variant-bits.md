---
section: performance
---
- **3D frames find their pipelines by a variant's bits** (zgx-3d): what sets a mesh's pipeline apart is one 16-bit number, built, compared and hashed as one, where it was eleven flags; which of light passing through, a coat or a specular a mesh has is kept as one byte as they change; and a pipeline's shader is written from an index of the shader's code made once, instead of every byte of it read again. `3d-mixed` falls 1.5% (148 K instructions a frame, 0.66 M of setup), `3d-textured` a little.
