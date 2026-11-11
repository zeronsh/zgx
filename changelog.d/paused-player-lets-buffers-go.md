---
section: performance
---
- **A paused video player gives back the pixel buffers it no longer shows** (zgx-video, zgx-gpu, macOS): a player shown as pixel buffers kept two past the one shown for as long as it was paused, 7 MB at 1280 by 720. It now gives them back once it has rested a few seconds (once a pause, never while playing), and its pool is flushed before CoreVideo's texture cache, which held a texture, and the buffer with it, of each buffer the pool still had.
