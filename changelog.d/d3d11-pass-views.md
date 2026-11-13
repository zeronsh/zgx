---
section: added
---
- **Direct3D 11 layer shaders read GPU views by name** (zgx-gpu, `d3d11`): a layer shader declaring GPU views among its inputs draws through them, as on Direct3D 12, instead of compositing its layer as painted. Each view's frame is painted at the box's size on the views' own Direct3D 12 device after the shader's passes, and Direct3D 11 waits for it on their shared fence before the frame samples it.
