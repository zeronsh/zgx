---
section: changed
---
- **Text sits on its baseline as Chrome puts it** (zgx): a line's ascent and descent are rounded to whole device pixels and its half-leading floored, as Blink lays out a line box, at every scale, so text, a line holding an inline box, and a span that is a box of its own (a keycap) land where a browser draws them. In 12-pixel Geist on 18-pixel lines the baseline is 13, not 13.25, and a keycap's top 14 above it, not 14.26, at 1x and at 2x. Lines can move by up to half a pixel.
