---
section: performance
---
- **A fade's bands are worked out once a frame** (zgx-gpu): a frame drawing a fade's bands as one worked each run's strips out twice, first to find whether any run needs them and again to draw it, over every item of the run (a list's hundreds of rows under an edge fade). The first run found keeps its strips for drawing, and the runs before it, found to need none, aren't asked again. Scrolling a deep transcript under glass and a fade runs 3.4% fewer instructions.
