---
section: performance
---
- **Turning on `scene3d` costs no node anything** (zgx-script): with the feature on, every node took about 12 instructions more as it lowered, a 3D scene on screen or not. Its `Kind` variant moved the others' discriminants and was tested on its own. It's the last variant now, tested with the kinds that need a component context.
