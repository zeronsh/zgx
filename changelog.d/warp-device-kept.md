---
section: fixed
---
- On WARP, a renderer made on one thread as the last one on another was dropped no longer crashes: WARP's one device is kept for the process, where a second device was made beside the first while it finished its last frame.
