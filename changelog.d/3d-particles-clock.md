---
section: fixed
---
- **3D particles lose no time to a painter still compiling** (zgx-3d, `particles` feature): a second painter of a world that drew first in a frame, its pipelines not made yet, took the frame's time without stepping the world, and the painter after it then skipped the step: particles lost a frame for every frame the new painter compiled. The world's particle clock now moves only when it steps.
