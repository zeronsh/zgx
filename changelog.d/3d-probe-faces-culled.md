---
section: changed
---
- **Probes drawn as faces cull what each face can't show** (zgx-3d, `gi`): where probes draw cube faces in place of tracing rays (Direct3D 12 under FXC, WARP, lavapipe, Vulkan or macOS without ray queries), each face now draws only the meshes, and the parts of big meshes, in its view within twice the probes' farthest counted distance (64 m at least), and a frame updates only as many probes as fit 16 million triangles. The harbor demo by faces went from 1.7 s a frame to about its frame time by rays, converging over more frames instead.
