---
section: performance
---
- **A glyph atlas grows without drawing its glyphs again** (zgx-gpu): growing copies the atlas into the larger one's corner and scales the texture coordinates of the quads already placed from it by the power of two between the sizes, which is exact, so they read as placing them fresh would. Before, growing rasterized every glyph on screen and placed every text's quads again. Archiving a message in the mail example, where the atlas grows, costs 22% less (paint 3.62 to 1.62 M instructions, 798 to 168 glyphs rasterized).
