---
section: added
---
- **A 3D world says what its device can do** (zgx-3d): `World::caps()` gives the view's device's `Caps` once a frame has been drawn: its tier (`Tier::A` on Metal, Direct3D 12 and Vulkan; `Tier::B` on GL), whether compute runs, storage buffers and textures per stage and in vertex shaders, indirect draws, timestamp queries, the compressed formats it samples, ray queries, texture arrays, and its compute limits. It's read once per device, before the first frame drawn with it.
