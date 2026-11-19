---
section: performance
---
- **A filling glyph atlas gives back only the glyphs no text holds** (zgx-gpu): the cells of glyphs no text the renderer holds any more are freed where they lie, and the rest stay where they are, so the text on screen keeps its glyphs and its placed quads; before, every fill started the atlas over, rasterizing and placing every visible glyph again (three quarters of that cost was the placing). The atlas packs cells in shelves that give cells back one at a time. Archiving a message in the mail example costs 9.6% less (paint 3.62 to 2.75 M instructions, 798 to 531 glyphs rasterized), and an atlas that fills again soon still grows as before.
