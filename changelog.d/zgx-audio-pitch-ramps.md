---
section: performance
---
- **A voice whose pitch moves, or whose sound is at another rate, costs a fifth less** (zgx-audio): a pitch ramp reads its frames in runs as a fixed pitch does, and the cubic interpolator gathers each frame's channels together. 64 voices ramping gain, pan and pitch cost 71.2 instructions a voice a frame (was 90.5); sounds at 44.1 kHz played at 48 kHz, 46.8 with the cubic (was 54.7) and 25.7 with the linear (was 26.5). Every sample is the same, bit for bit.
