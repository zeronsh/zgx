---
section: performance
---
- With Direct3D 11 (`ZGX_D3D11`), a grown glyph atlas shrinks at idle by copying its glyphs across, as with Direct3D 12, instead of starting over: the first frame after a long idle no longer rasterizes the screen's text again.
