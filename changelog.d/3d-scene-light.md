---
section: added
---
- **`scene_light(uv, lod)` in custom materials** (zgx-3d, `shaders` feature, in a shader marked `.reads_scene()`): the scene behind as light, before exposure and tone mapping, whether the post chain is on or off, so glass and water give back what is behind them once mapped, never twice. With the chain off it is the displayed scene taken back through the world's tone map (Neutral, ACES or Linear) and exposure: exact below white, lossy above.
