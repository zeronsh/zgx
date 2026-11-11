---
section: fixed
---
- **A colour map's smaller levels keep its brightness** (zgx-3d): the levels made below a texture's pixels (an app's own, a PNG's or JPEG's, a one-level KTX2 file's) average a base colour or emission map's texels in linear light, where before they averaged the sRGB bytes, so fine contrast went dark with distance: a 1-pixel black and white checker's far levels are now 188 (half the light), not 128 (a fifth). Maps read as data (normal, metal and roughness, occlusion…) average as stored, as before, so a normal's mean keeps its length. A side of odd length counts its last row or column, where before it was dropped.
