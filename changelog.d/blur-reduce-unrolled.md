---
section: performance
---
- **A frosted panel costs the GPU a fifth less a frame**: its backdrop blur's reduce pass took each texel's samples in a loop of a count known only at run time, each waiting on the last. They are now written out for each factor, the same samples in the same order, so the pixels are the same to the byte: a 1280 by 110 frosted bar over a scrolling list, 661 to 538 µs of GPU a frame at 2x, 1,338 to 1,099 at 3x (M5, Metal).
