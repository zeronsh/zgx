---
section: performance
---
- **A frame where a few objects moved works out only their places** (zgx-3d): objects' places in the world are kept from frame to frame, walked parents first, and made again only for objects changed since, or under one. The order they're walked in is found again only when the hierarchy changes (an object added, removed or put under another), not for a material or a hide. In a 9,331-object hierarchy a frame moving one branch costs 13% less, one moving a leaf or changing one material 13 to 17% less; the demo's frames with its cranes working 4 to 5% less; 3d-anim 6% less. Memory at rest is as before (80 bytes an object).
