---
section: added
---
- **Direct3D 11 layer shaders read pictures by name** (zgx-gpu, `d3d11`): a layer shader declaring images among its inputs (`zgx_<name>`, `_texel`, `_size`) draws through them, as on Direct3D 12, instead of compositing its layer as painted. Only a layer shader reading a GPU view is still drawn as painted.
