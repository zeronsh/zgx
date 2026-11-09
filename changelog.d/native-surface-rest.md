---
section: fixed
---
- **A window at rest lets go of a native surface it no longer shows** (zgx-gpu, macOS): CoreVideo's texture cache kept the last one (a video player's last picture, after the player was gone) for the window's life, since nothing flushed it once frames stopped. Going idle flushes it now, once: no timer, and no wakeups at rest. A 1280 x 720 picture: 3.3 MB less at rest.
