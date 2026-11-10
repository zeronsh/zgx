---
section: added
---
- **A 3D world runs the app's compute shaders over its own GPU buffers and textures** (zgx-3d, `compute` feature): `World::add_buffer` and `add_texture` give handles the app owns, counted in `World::memory()` under a cap; `World::add_compute` runs WGSL with them bound by name every frame, at a rate or on demand, before or after the frame, ordered by what each writes and reads. Shaders are checked when added, errors at the app's own lines, and compiled off the frame. `buffer.read(range)` and `texture.read()` come back a few frames later within a per-frame budget, never stalling one. A world with nothing due draws nothing.
