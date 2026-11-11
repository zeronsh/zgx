---
section: fixed
---
- **3D particles with sub-emitters draw the same each run** (zgx-3d, `particles` feature): on the GPU, a frame's births (splashes where rain lands) took their slots in whatever order the GPU ran their parents, so which were kept when a frame's births passed the pool, which older ones they replaced and the order they drew in changed run to run. Births are now placed in their parents' order: a world drawn at the same frame times draws the same pixels.
