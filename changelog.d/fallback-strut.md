---
section: fixed
---
- **A line of fallback glyphs stands on its text font's baseline** (zgx-gpu, zgx-cosmic-text): a run that falls back keeps its default family's face as the line's strut: the fallback glyphs add to the line's ascent and descent, and the default face's own glyphs, and runs that never fall back, are as they were. Before, a line of Japanese or Chinese alone took its fallback face's metrics and could sit a pixel off the baseline the same text's Latin lines stand on.
