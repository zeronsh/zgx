---
section: performance
---
- **A 3D world's bloom runs as one compute pass where the device has compute** (zgx-3d, `post` feature), and where the device stores to half-float textures from compute (`Caps::stores_half_floats`, asked of the device): its ten steps at 1920 by 1080 were a render pass each, and on Metal each render pass a frame holds about 8 MB of driver memory at rest. As dispatches of one compute pass, drawing the same frame (the steps' taps shared with the render path, which devices without compute keep), the post chain with bloom and a grading table rests 89 MB over the chain off rather than 122, and costs 0.075 ms of GPU time a frame rather than 0.41, on an M5.
