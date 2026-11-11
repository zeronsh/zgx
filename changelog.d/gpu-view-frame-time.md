---
section: fixed
---
- **A GPU view's frames are on the interface's time, one instant a frame** (zgx-gpu): `ViewportFrame.time` was the wall clock's at each view's draw, so a 3D world in a video made with a virtual clock (zgx-video) drew each frame at whatever time the machine took to make it, and a view shown in two boxes was drawn at two instants a frame (what steps once a frame time stepped twice). It's now `zgx::clock::now()` (the virtual clock's while one runs), read by the first view drawn in a frame and given to every view and box of that frame.
