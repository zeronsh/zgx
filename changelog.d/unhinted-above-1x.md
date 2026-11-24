---
section: changed
---
- **macOS text above 1x is drawn unhinted, as Core Text draws it** (zgx-gpu, zgx-text): glyphs at 2x (Retina) were hinted by swash, whose autohinter (skrifa's, on faces with no instructions such as SF) is none of macOS's. Core Text hints nothing at 2x, and unhinted glyphs now match its own to a mean of 0.02 of a pixel's coverage, where hinted ones were 0.13 apart. Text at 1x keeps swash's hinting until CoreGraphics' own autohinter, which runs there, is matched. Every glyph at 2x moves slightly: check screenshots compared pixel for pixel.
