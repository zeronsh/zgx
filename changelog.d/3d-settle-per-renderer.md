---
section: fixed
---
- **Two views of one 3D world each finish what they're converging** (zgx-3d): what keeps a world drawing (probes tracing, exposure adapting, cascades catching up, maps loading, shaders compiling) is now held by each renderer drawing it, as `World::view()`'s and `World::viewport()`'s are, and the world draws on while any of them holds a reason. Before, whichever drew last in a frame decided: one view finished could stop the world while the other was still part-way. Each lets its reasons go as it rests or is dropped.
