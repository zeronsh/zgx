---
section: fixed
---
- **A box shadow's spread on a superellipse corner is the box's true offset** (zgx-gpu): the spread ring was drawn as a corner `spread` larger, which drifts off the box at the diagonals (a 1px `0 0 0 1px` ring showed dark corner bands); it is now the box's distance less the spread, so the ring has the same width all round. Round corners are unchanged. (draw.hlsl for Direct3D 11 needs `scripts/d3d11_shaders.py` run on Windows.)
