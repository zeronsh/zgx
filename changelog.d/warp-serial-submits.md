---
section: fixed
---
- On WARP, wgpu devices of their own on several threads of one process (zgx-3d's, GPU views', an app's) no longer crash or draw nothing when they submit at once: their submissions take turns.
