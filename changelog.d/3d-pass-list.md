---
section: changed
---
- **A 3D world's frame is a compiled list of passes, and a world at rest lets go of its targets** (zgx-3d): the shadow, glass and mesh passes say what they read and write, and their targets come from a pool, compiled when the world's settings or size change rather than each frame. When the window goes idle (`Viewport::rest`), a world drops its multisampled colour, depth and what glass reads, and makes them again with its next frame. A world draws as it did.
