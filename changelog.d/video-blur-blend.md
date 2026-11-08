---
section: performance
---
- **Motion blur mixes a frame's samples in a sixth of the instructions** (zgx-video): sums are 16 bits up to 256 samples, the first sample is written rather than added to zeros, and the average shifts for a power of two of samples or multiplies by an exact reciprocal for other counts, where it divided each byte. Frames are byte-identical; the showcase's 4-sample render went from 387 G to 243 G instructions, its 3-sample one from 327 G to 198 G.
