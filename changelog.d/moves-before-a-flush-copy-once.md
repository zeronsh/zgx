---
section: performance
---
- **Several scroll moves before a frame copy once by the net move** (zgx): when a fast wheel or trackpad sent two or more notches between two refreshes, the scroll view repainted its whole clip that frame instead of copying. It now copies by the net move and paints the strip exposed, with any damage made while the content sat between two moves shifted to where it ends. A move back where it was copies nothing and paints only what changed; a move past the view, or more than three moves in a frame, repaints the clip as before.
