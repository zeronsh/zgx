---
section: performance
---
- **A frame with a backdrop blur makes its bind groups once, not every frame** (zgx-gpu): it made the blur's bind groups (up to five) and two viewport groups anew each frame, each a trip through wgpu's validation, because they bound ranges of that frame's vertex buffer. The blur's records are now bound at dynamic offsets and its groups kept; the viewport's groups are kept by where the frame's regions lie, which a frame whose quads kept their size shares with the last; Direct3D 11 keeps its blur records buffers from frame to frame; and the frame reads the device's storage alignment once, not through wgpu's limits each frame. A scrolled frame under a frosted panel makes one bind group where it made seven, and about 93 fewer allocations (of some 490).
