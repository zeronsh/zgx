---
section: fixed
---
- **Glass reads what's behind it with other glass in the frame** (zgx-gpu): with several layer shaders reading the backdrop (`zgx_backdrop`) changing at once, a frame could draw holes and wrong backdrops between and under them, on Metal and Direct3D 12. The frame grew its damage over what each reads, but found its items only within the damage before that.
