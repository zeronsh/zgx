---
section: performance
---
- **A 3D world leaves out the meshes its camera can't see** (zgx-3d): each mesh's box in the world (the one the sun's shadow map is fit to) is tested against the view's six planes, and the meshes' pass draws each run's visible instances in spans, in the order they had, so a frame is exactly what it is with culling off. A gap of meshes out of view is drawn with its span where it holds fewer indices than a draw costs (measured on an M5 Max: a draw 142–147 ns, an off-screen index 0.026–0.045 ns), and culling rests for 15 frames after one where it left out under 2%. Shadow casters are all still drawn.
