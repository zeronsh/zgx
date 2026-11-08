---
section: performance
---
- **A window back from a long idle draws its text from cache, with no hitch** (zgx-gpu): the glyph atlas a window grew no longer starts over empty at rest. It shrinks to the smallest atlas holding the glyphs and icons on screen, copied across rather than rasterized again, and an atlas the screen fills stays. On an M5, a page of CJK text whose atlas had grown to 16 MiB rests at 4 MiB (the screen's 600 glyphs), and the first whole repaint after the idle draws in 0.25 ms of paint instead of 9 ms (114–126M instructions to 0.6M).
