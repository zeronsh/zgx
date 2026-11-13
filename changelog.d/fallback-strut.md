---
section: fixed
---
- **A line of fallback glyphs stands on its text font's baseline** (zgx-gpu, zgx-cosmic-text): a line's ascent and descent now include the font the text asked for as well as the fonts of its glyphs, as a browser's strut does. Before, a line of Japanese or Chinese alone took its fallback face's metrics and could sit a pixel off the baseline the same text's Latin lines stand on.
