---
section: changed
---
- **Text tracks and sits on its baseline as a browser draws it** (zgx-gpu, text): a font's `trak` table (SF's tracking) is read at the size the text is drawn, not at 12 points, and a line's baseline takes its ascent and descent rounded to whole pixels with the half-leading floored, as Blink places it. Lines with a fractional ascent or an odd leftover space sit up to a pixel differently, now on a whole pixel.
