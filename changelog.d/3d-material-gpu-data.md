---
section: added
---
- **A custom material reads an app's GPU buffers and textures** (zgx-3d, `shaders` and `compute` features): `MaterialShader::buffer(name, wgsl_type)` and `Param::GpuTexture` bind a `GpuBuffer` or `GpuTexture` by `World::set_param`, so a compute pass's output (water's heights, a simulation's state) feeds a material directly. A device change binds the data's new copy.
