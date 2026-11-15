---
section: performance
---
- **A terminal row splits into its runs more cheaply** (zgx-terminal): telling where a run's style ends compares every cell's two colours with its neighbour's, now as numbers (a named colour by its index, an RGB by its three bytes at once). A many-coloured log scrolling through the engine-drawn terminal takes about 1.4% fewer instructions.
