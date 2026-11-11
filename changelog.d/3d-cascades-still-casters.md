---
section: performance
---
- **A 3D caster moving in the sun's cascades redraws itself alone** (zgx-3d, `lighting` feature): each cascade holding a mover keeps a layer of its other casters (those unchanged for 120 frames), and a frame where only movers changed draws the cascade from that layer and the movers, not every caster it holds again; a far cascade a mover touches as the camera turns keeps its fit until its turn to follow. In the harbour example with its cranes working (1080p, M1 Max) the cascades draw 2.7× fewer caster instances a frame, and the GPU's frame is about 0.55 ms (5%) shorter, at the same CPU cost within its spread. The layers are made once a caster moves: 32 MB more for four cascades of 2048.
