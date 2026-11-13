---
section: fixed
---
- **A line of fallback glyphs stands on its text font's baseline** (zgx-gpu, zgx-cosmic-text): a run that falls back keeps its default family's face as the line's strut: the fallback glyphs add to the line's ascent and descent, and the default face's own glyphs, and runs that never fall back, are as they were. Before, a line of Japanese or Chinese alone took its fallback face's metrics and could sit a pixel off the baseline the same text's Latin lines stand on.
- **When the default family isn't installed, the system's own font is the strut**: the sans-serif family (Open Sans here) missing, a line of fallback glyphs alone stood on its fallback face's baseline because there was no default face to take; the first common fallback face that exists (macOS's `.SF NS`) now is.
- **macOS's sans-serif (and `system-ui`) is the system face** (zgx-gpu): the generic family resolves to `.SF NS` (SFNS.ttf) where it is installed, as Chrome maps it, instead of cosmic-text's Open Sans that no Mac has, so the text's own font no longer comes through the fallback path.
