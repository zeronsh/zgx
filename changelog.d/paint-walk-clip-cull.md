---
section: performance
---
- **A repaint draws what its clips show, not every node** (zgx): the paint walk passes by children whose ink falls wholly outside the clip they're drawn in (a scroll view's rows out of view), on a full repaint as on a damaged one. Resizing a 3,000-row file list visits 144 nodes a frame instead of 4,715, and its paint phase falls 36% (7.43M to 4.72M instructions).
