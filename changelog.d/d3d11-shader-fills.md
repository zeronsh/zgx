---
section: added
---
- **Direct3D 11 draws shader fills and layer shaders** (zgx-gpu, the experimental `d3d11` feature under `ZGX_D3D11=1`): an app's `FillShader`s and layer shaders that don't read the backdrop, `radial` gradients, `hue_rotate`, `sepia` and `invert`, and zgx-fx's fills and layer effects draw as they do on Direct3D 12, byte for byte, where a frame holding one failed before. Each one's pixel stage compiles off the frame from the same module, by FXC in a process of its own; until it has, a fill draws its box's colour and a layer as painted. CI now runs Direct3D 11's parity test on Windows, every ready-made effect compiled cold.
