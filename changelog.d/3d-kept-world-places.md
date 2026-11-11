---
section: performance
---
- **A frame where a few objects moved works out only their places** (zgx-3d): objects' places in the world are kept from frame to frame, walked parents first, and made again only for objects changed since, or under one. In a 9,331-object hierarchy a frame moving one branch costs 13% less, one moving a leaf 16% less; the demo's frames with its cranes working 4 to 5% less; 3d-anim 7% less. Memory at rest is as before (80 bytes an object).
