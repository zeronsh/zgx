---
section: added
---
- **A textarea's atom can show as a picture (an equation)** (zgx, zgx-script): `atom_style.svg = { src | code, width, height, descent }` in EM (and an atom's own `style.svg`) shows its text as one box of that size with the picture in it; one caret unit, its line as tall as it needs, its text kept for copy, search and assistive technology. A line holding only it centres and pads as a display equation with `LineInset { align, above, below }` (Luau internal; `AtomStyle::svg`, hidden, in Rust).
