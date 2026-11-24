---
section: performance
---
- **Fade bands size what they build once** (zgx-gpu): a run of items under one fade mask (a list's rows under an edge fade) was gathered push by push, and so were its band's items, each growing about ten times a frame for hundreds of rows; each is now made at its length, and a run's texture keys are gathered only for a run with bands to draw. Banding is worked out out of line, so a frame with nothing faded runs the code it ran. A scrolled frame under a frosted, faded panel reallocates 28 times fewer, and scroll-glass runs 1.9% fewer instructions.
