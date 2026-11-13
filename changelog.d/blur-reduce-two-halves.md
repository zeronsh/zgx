---
section: performance
---
- **A frosted panel holds 32 MB less on macOS, and its blur's first pass takes half the GPU time** (zgx-gpu): a backdrop blur wide enough to average its area down by 8 did so with 16 samples in flight per pixel, which held so many registers that Metal's driver kept four 8 MB blocks of scratch for as long as the window drew. It now averages down in two halves of four samples (the first into a texture at twice the slot's size), on every backend. A pixel of the blur may differ by 1 in 255 from before.
