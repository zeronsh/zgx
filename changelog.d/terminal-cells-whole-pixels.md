---
section: fixed
---
- **A terminal's cells are whole device pixels** (zgx, zgx-terminal): a cell was the font's advance exactly (15.6 pixels at a scale of 2), so columns' edges and glyphs fell at a different fraction of a pixel across a line. Its width is now rounded and its height rounded up to device pixels, and measured again when the window's scale factor changes; on a 2x Mac the grid is a little wider (8 points a cell for 7.8). `NodeHandle::observe_text_geometry` and `Scene::pixel_scale` are public, for views that measure text by device pixels.
