---
section: fixed
---
- **A 3D world's shaders keep one clock** (zgx-3d): custom materials' and post passes' `frame.time`, `frame.delta` and `frame.index` come from one clock per renderer, read from the frames' own times, so a material's wave no longer jumps back to its start when the post chain turns on or off or the samples change, and a material and a post pass agree on the time. With post passes in, changing the samples no longer leaves them drawing with the targets from before; removing the last pass lets go of the textures it had bound.
