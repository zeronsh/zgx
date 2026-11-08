---
section: fixed
---
- **Layout settles when an observed position only rounds differently, or a size is unbounded** (zgx): observed bounds (`ViewHandle::observe_bounds`, what anchors a popover or a tooltip) are told within a 1024th of a point, as sizes are since #1027, so a position flipping between neighbouring floats under a reader that moved it no longer ran out the frame's layout passes. An infinite size was told again on every pass (infinity less itself is NaN, near nothing) and never settled; it is the same size now.
