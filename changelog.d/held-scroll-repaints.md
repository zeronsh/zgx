---
section: fixed
---
- **A view scrolled while a Wayland frame waits is painted whole with the next** (zgx-desktop): a scroll's flush damages only what copying the view's old pixels can't give (a fading view's bands, the strip scrolled in), for a copy over the frame drawn from the flush before. A frame-paced window held that flush's damage for the frame the compositor granted next, but not its move, so a view whose next flush moved nothing (its rail, a pill, a hover changing beside it) or whose moves fall between device pixels (1.25x, 1.875x) kept its old pixels: every row of a transcript off by as far as it had scrolled, torn against its freshly drawn bands, until something repainted it. The held frame now holds each view it scrolled.
