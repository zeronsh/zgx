---
section: performance
---
- **A steady trackpad stream follows the fingers sooner** (zgx-desktop): after eight regular intervals, input arriving at a multiple of the display rate gradually uses half the sampling delay. Sparse, irregular and mismatched input retains its interpolation margin; wheel easing and the number of frames requested are unchanged. No motion is predicted beyond the input received.
