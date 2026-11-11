---
section: added
---
- **GPU views can trace rays** (zgx-gpu): `unsafe { zgx_gpu::request_ray_queries() }`, called before the first window, makes the shared device ask for wgpu's ray queries (`EXPERIMENTAL_RAY_QUERY`) and acceleration-structure limits where the adapter has them, so a view's painter can build acceleration structures and trace rays from compute and fragment shaders. Apps that don't call it get the same device as before. `zgx_gpu::ray_queries_status()` says whether the device traces rays, and if not why: not requested, the adapter has none, or Direct3D 12 compiling with FXC.
