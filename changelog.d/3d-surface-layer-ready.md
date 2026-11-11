---
section: fixed
---
- **`World::shader_ready` says when a surface layer has taken effect** (zgx-3d, `shaders` feature): it stayed false for a layer that was drawing. A layer is now ready once a frame has made the stock pipelines with it and none of a mesh shader's pipelines (made with every layer) is still compiling.
