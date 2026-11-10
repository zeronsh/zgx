---
section: performance
---
- **A text that changes as it is drawn no longer trims the text cache every frame** (zgx-gpu): past its 64 KB budget of text no longer drawn, the cache now keeps half of it rather than all, so the next changed text doesn't put it straight back over. A 54-character label changing every frame (a counter, a header rolling its count) trims once in 7 frames instead of every one, and a frame that changes one text renders in about 93K instructions instead of 124K (−25%, callgrind, `micro_bench SCENE=state`). What it holds still comes back down at rest.
