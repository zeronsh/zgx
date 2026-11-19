---
section: performance
---
- **The software raster parses its bundled font when text first needs it, once for the process** (zgx-desktop): every window drawn on the raster parsed DejaVu Sans with fontdue as it was made, every glyph's outline (about 17 MB of heap), though only text drawn without the scene's text services falls back to it. It is parsed the first time a raster draws with it, and shared by every raster in the process.
