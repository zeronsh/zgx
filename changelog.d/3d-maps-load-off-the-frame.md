---
section: performance
---
- **A textured 3D model shows at once and its maps fill in as they load** (zgx-3d): maps read from files are decoded, given their smaller levels and put on the GPU by worker threads (the cores less two, at most 8, parked when idle), and a mesh draws in its material's own colours until its map arrives; the world draws frames until every map is in, then rests. `World::loading()` says whether any is still on its way. 64 textures of 1024² (M1 Max): first frame at 30–50 ms, every map in at 126–149 ms, against 182–553 ms for the frame that waited for all of them. Maps of 128² or less, maps given as pixels, and frames drawn whole (a video's) still wait.
