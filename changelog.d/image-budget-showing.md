---
section: fixed
---
- **A new picture past the texture budget no longer blanks one on screen** (zgx-gpu): a picture a frame painted from what it kept wasn't marked as drawn, so one on screen could give its texture to a new one and stay blank, the frame reporting success. Those the scene still shows are spared now, worked out only when a texture has to give way: the new one goes softer, as designed, and pictures off the window still give way.
