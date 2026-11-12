---
section: added
---
- **A canvas draws radial and conic gradients** (zgx, zgx-gpu, zgx-script): `ui.radial(cx, cy, r, colors, { ry, transform })` runs out from a centre to a circle or ellipse, the last colour past it; `ui.conic(cx, cy, colors, { angle, transform })` turns clockwise round a centre from `angle` degrees off +x. `transform` (`{ a, b, c, d, e, f }`) maps either as a drawing's own transform does, so a rotated or mirrored shape's gradient turns with it. Both go wherever `ui.linear` does, a box's `bg_fill` too. In Rust, `Brush::radial` and `Brush::conic` place the unit circle by an `Affine`.
