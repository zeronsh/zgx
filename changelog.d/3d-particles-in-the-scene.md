---
section: changed
---
- **3D particles draw with the meshes, against their depth** (zgx-3d, `particles` feature): a mesh in front of an emitter now hides its particles, which were drawn over the whole frame. They're multisampled, exposed and tone-mapped with the meshes, and with the post chain on they bloom like the rest of the scene. Additive particles draw before translucent meshes (glass in front tints a glow) and alpha-blended ones after them.
