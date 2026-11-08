---
section: fixed
---
- **A layer shader reading the backdrop sees all of it on macOS** (zgx-gpu): now and then 16-pixel blocks of what it read were the backdrop from before this frame's copy, showing black or flat bright colour where it had not read before. Apple GPUs' lossless compression let draws sampling the backdrop miss part of a copy into it ordered before them; the backdrop is now made without it.
