---
section: performance
---
- **A custom fill never freezes the frame that first draws it**: a `bg_shader`'s pipeline compiles on a thread of its own, from the moment `ui.shader` declares it. A window's frame that draws it before it's ready shows the box's colour and draws again once it is, instead of waiting. The first frame drawing Zeron's status ring took 117–151 ms on an M-series Mac (the frame waited for the compile); it takes 0.5 ms, and the ring follows about 150 ms later. Offscreen renderers (snapshots, tests) still wait, so their pixels stay deterministic.
