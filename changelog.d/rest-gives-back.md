---
section: performance
---
- **A window at rest keeps what it shows, and gives back the rest** (zgx, zgx-gpu, zgx-script, zgx-desktop): as a window goes idle, text keeps only what is drawn, slots past the last node in use go from the scene and its effects, rasters no node shows go, and markdown streams not asked for in a minute. The glyph atlas goes after 6 s at rest, not 30.
