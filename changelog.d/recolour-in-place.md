---
section: performance
---
- **A recoloured text's kept glyph quads take their new colours in place** (zgx-gpu): rich text laid out as it was with its runs' colours changed (a fade's frame, a highlight) was placed again glyph by glyph from the atlas (a lookup each) with its decorations; its kept quads now take each one's run or decoration colour where they lie, out of line from the paint walk, which asks one test of each text as before. Text whose quads aren't kept (a twin's copy, a new atlas) is placed again as before.
