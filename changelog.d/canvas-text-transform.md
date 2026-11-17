---
section: added
---
- **Canvas text turns with a transform** (zgx, zgx-script): `c:text(text, x, y, { transform = { a, b, c, d, e, f } })` maps the placed text by that matrix (SVG's `matrix` order), so a label in a rotated, mirrored or scaled frame turns with the frame's shapes instead of staying upright. In Rust, `Canvas::text_transformed` adds one and `Canvas::text_transform` says what a text is mapped by; its node takes the transform's scale, skew and turn about its top left.
