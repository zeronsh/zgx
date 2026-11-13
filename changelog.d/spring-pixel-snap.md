---
section: fixed
---
- **A box a spring or tween moves with `x` or `y` rests on a whole device pixel** (zgx-script): its translation is rounded to device pixels, as scroll content is, so a fractional target no longer leaves its edges and text between pixels. Needs no rounding in Luau. `View::pixel_snap` does the same for a Rust view.
