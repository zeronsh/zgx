---
section: fixed
---
- **A hard ring round a box leaves no seam where the box's edge falls inside a pixel** (zgx-gpu): a shadow ring (`0 0 0 1`, Metal's edge) is cut out of the box that is drawn over it; where the edge lay inside a pixel, both anti-aliased it and the pixel kept three quarters of its colour, a line of what is behind between ring and fill (a centred composer, a fractional height, a zoom). The ring now tucks half a device pixel under an opaque flat fill.
