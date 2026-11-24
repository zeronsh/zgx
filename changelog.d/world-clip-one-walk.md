---
section: performance
---
- **What shows of a node is worked out in one walk up** (zgx): `visible_bounds` and `shows_within`, read for every observed node and every list scrolled with the view around it, walked a deep node's ancestors once for its place and opacity, then again for its clips, and again from each clipping box for that box's own place. One walk now does it all, each clipping box summing its place on the way, the same sums in the same order, so the results are bit-identical. Damage keeps the two short walks, which cost it less.
