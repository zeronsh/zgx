---
section: changed
---
- **Text below 2x draws on whole pixels, with firmer edges** (zgx-gpu): on a screen at 100% to 175% (most Windows and Linux monitors), each glyph now goes on the nearest whole device pixel instead of a quarter-pixel offset, and its mask's edges are firmed by how light the text is, as DirectWrite's enhanced contrast does (near-black text on white keeps its weight). A vertical stem a quarter pixel off spread over two columns at part strength, which read as blur next to the system's own text. Layout, line widths and wrapping are unchanged; a glyph moves by at most half a pixel. At 2x and over, text draws as before.
