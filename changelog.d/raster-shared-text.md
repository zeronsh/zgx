---
section: added
---
- **The software raster can draw plain text as the GPU renderer does** (zgx-desktop, zgx-gpu): given the font system and text cache its scene is measured with (`Raster::share_text`), it lays out plain text from that cache and draws the GPU's glyph images on the GPU's pixels, with the same coverage firming and underlines (`zgx_gpu::plain_glyph_places`, `glyph_image`, `glyph_coverage`). Its frames match the GPU's to within blending's rounding: no channel off by more than 4 of 255 (1 on Metal); before, 5.9% of a text list's pixels differed and lines wrapped where layout hadn't. Without it, the raster draws as before.
