---
section: performance
---
- **Direct3D 11 backdrop readers apart share one copy** (zgx-gpu, `d3d11`): layer shaders reading the backdrop that needn't see each other read one copy of the target, as on Direct3D 12, instead of a copy each.
