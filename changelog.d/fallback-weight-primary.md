---
section: fixed
---
- **A system font reached because the family asked for isn't installed follows the weight axis** (zgx-cosmic-text): the snap to a named instance (450 as Medium) now applies only to a face a glyph falls back to while a face of the family asked for exists. With none installed (`system-ui`, a web font's name before it loads), the system font found is the text's own, and draws and measures at the weight asked as Chrome's does (503.4, 508.7, 514.2 at 14 px, 400, 450, 500, matching Chrome to 0.01): before, 450 and 500 both measured as Medium, a few points wider than Chrome on a line.
