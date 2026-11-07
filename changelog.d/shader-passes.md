---
section: added
---

- `ui.layer_shader(wgsl, { passes, images, geometry })`: a shader can declare passes that draw textures before each frame (persistent, in floats, with feedback and steps, or once), sample pictures, and read where tagged boxes are (`geometry = "tag"`), all by name. `zgx_window(p)` gives a shader window points. Rust gets the same as `FillShader::with_inputs`, and `ShaderInputs::views` feeds a shader from your own wgpu passes. A shader without inputs pays nothing.
