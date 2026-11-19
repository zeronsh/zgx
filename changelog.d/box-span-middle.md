---
section: added
---
- **A box span can sit at its paragraph's middle, with room beside it** (zgx, zgx-script; internal, may change): `TextSpan::vertical_align(InlineAlign::Middle)` (Luau `vertical_align = "middle"`) puts a span that is a box of its own (a keycap) as CSS's `vertical-align: middle` does, its box's middle half its paragraph's x-height above the baseline, its text with it and its line growing to hold it; `TextSpan::margin` (Luau `margin = 2` or `{ left, right }`) keeps room beside it outside its box. The x-height is a font's as Blink takes it: its OS/2 table's, else its `x`'s height, else half an EM.
