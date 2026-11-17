---
section: fixed
---
- **Letters below 2x are evenly spaced again** (zgx-gpu): each glyph went on the whole device pixel nearest its own place, so where a font's advances fall between pixels (nearly always) a letter stood a pixel nearer or further from the next than its layout says, and the gaps inside a word were visibly uneven at 100% to 175%. A line's start still goes on a whole pixel, so a line that only moves keeps its spacing, and its glyphs keep their quarter-pixel places from there; masks stay firmed.
