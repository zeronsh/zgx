---
section: added
---
- **Direct3D 11 draws GPU views** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): zgx-3d's worlds, `scene3d` and other `gpu_view` painters drew empty there. They now draw with wgpu on a Direct3D 12 device of the views' own, on Direct3D 11's adapter, into textures shared with Direct3D 11 behind a shared fence. Painters change nothing; `viewport::view_device_features` lists what that device asks for where the adapter has it.
