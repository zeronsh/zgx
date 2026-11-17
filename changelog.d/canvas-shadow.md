---
section: added
---
- **A canvas draws shadows** (zgx-script, zgx, zgx-gpu): `c:shadow(path, { x, y, blur, spread, color })` draws the shadow a path casts, written as a `shadow` property is (CSS's blur), under what is drawn after it: a card's drop shadow is `c:shadow(card, …)` then `c:fill(card, …)`. Any path casts one, blurred as a text shadow is and cut by the canvas's clip; `spread` grows it, or shrinks it below 0. In Rust, `Canvas::shadow` and `Draw::Shadow`. A canvas with a shadow rasterizes, as one with a gradient does.
