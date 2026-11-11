---
section: performance
---
- **The first code block a window shows parses one font, not every fixed-width one** (zgx-cosmic-text): asked whether the monospace family's face covered a word, a build without `monospace_fallback` found no face covering anything, so the first monospaced word parsed every fixed-width face on the system (13 on a Mac) before settling on that face anyway. Its character map answers now: a window's first frame with code in it is about 0.3–0.6 ms sooner, with the same glyphs.
