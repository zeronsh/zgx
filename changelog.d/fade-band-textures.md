---
section: fixed
---
- **A fade over stacked pictures holds textures the size of its bands, and none at rest** (zgx-gpu): where `fade_edges` crosses stacked items, the renderer draws them unfaded into a band texture and fades it once. One texture covered every band, so a clip faded at its top and bottom took one the size of the whole clip (Zeron's wallpaper, a plugin's layered scene: a 1888×1600 texture, 11.5 MiB, reported as a layer on a node that isn't one), and the pool of band textures was never let go. Bands apart now each take a texture of their own size (1.9 MiB there), and the textures go as the window goes idle (`GpuRenderer::trim`).
