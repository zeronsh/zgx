---
section: fixed
---
- **A percentage width in a sideways scroll's content is of its viewport** (zgx): `min_w = "100%"` on a row in a `scroll_x` (display math centred, scrolling when wider than the answer) spans the viewport, as in CSS, and wider content still scrolls. A scroll also grows in flow along an axis only when it has a cap on that axis, so a vertical scroll with only a `max_h` keeps a definite width for its content's percentages.
