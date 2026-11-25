---
section: performance
---
- **A script's shader is checked in a smaller module first** (zgx-gpu, zgx-script): `ui.shader` and `ui.layer_shader` check their source in the module the renderer builds around it, and parsed and validated all of that module, the renderer's own part too, for each shader. The renderer's functions are left out of it where the source doesn't name them; a source that isn't known to be right so is checked whole, so errors say what they did. A source already made into a shader isn't checked again (a script reloaded, another window, a video's next stage).
