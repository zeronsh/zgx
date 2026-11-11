---
section: fixed
---
- **A 3D world's particles step once a frame, however many painters draw it** (zgx-3d, `particles` feature): with a second painter of the same world (`World::viewport`, a thumbnail beside the window), each painter added its own step to the world's particle time, so particles aged and spawned at twice the rate, and the second painter's pools never got the bursts. The world now steps once per frame time, whichever painter draws first, and every painter on one device draws the same pools.
