---
section: added
---
- **A custom material knows whether the scene behind it is light** (zgx-3d, `shaders` feature): `scene_is_light()` is true where the post chain is on, where `scene_color` is the scene's light before exposure and tone mapping and a material gives it back as it is. Where the chain is off, `scene_color` is the scene as displayed, already mapped once. The docs said "as displayed" in both cases.
