---
section: fixed
---
- **A 3D world's compute steps once a frame, however many places show it** (zgx-3d, `compute` feature): with its view shown in two boxes, every-frame passes ran twice a frame, so a simulation ran at a speed that depended on how many places showed it. A second painter of the same world (`World::viewport`, a thumbnail beside the window) made the app's buffers and textures again every frame, zeroed. Passes now run once per frame time, and every painter on one device shares the world's GPU data.
