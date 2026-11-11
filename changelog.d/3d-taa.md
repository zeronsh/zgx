---
section: added
---
- **A 3D world can smooth its edges with time: temporal antialiasing** (zgx-3d, `post` feature): `World::set_antialiasing(Antialiasing::Taa)` draws one sample a pixel, jittered over eight offsets, blended with the reprojected history (clamped in YCoCg to the frame's neighbourhood, Catmull-Rom history reads, depth-tested so what moved doesn't trail, blended in the tone curve's space). A still view settles finer than 4x multisampling in 8 frames and stops drawing; under camera motion still meshes leave no trail. `World::cut()` drops the history. Until the depth prepass, frames in which meshes moved lean on themselves (moving meshes aren't smoothed).
