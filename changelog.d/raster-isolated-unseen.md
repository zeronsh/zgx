---
section: performance
---
- **The software raster draws a layer only where its frame repaints it** (zgx-desktop): a group drawn as a layer (faded as one, transformed, blended, masked) was drawn whole into two mattes for every area a frame repainted, even one far from it, and each matte copied the bundled font, every glyph's outline. A layer away from the repaint now costs nothing, and the mattes share the font. A terminal window's close button is two rotated strokes: under output, each frame paid for them though only the terminal's rows changed.
