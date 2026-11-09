---
section: performance
---
- **Scrubbing back in a video player's view shows kept frames at once** (zgx-video, macOS): a player's own view keeps the pixel buffers of frames shown while paused, as a script's player keeps their bytes, so a frame scrubbed to again is shown as it is rather than played again from the composition's start. Scrubbing back over 20 frames of the bench composition at 640 x 360, a second time: 25 ms a frame to 0.3 ms at frame 300, and 55 to 70 ms to 0.3 ms at frame 800. The buffers count in the same 512 MB as bytes, and at rest are given back as the bytes are (5 s), with the pool's spare buffers: the process's footprint at rest is no larger than before.
