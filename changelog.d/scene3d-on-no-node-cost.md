---
section: performance
---
- **Turning on `scene3d` adds no test for every node** (zgx-script): with the feature on, `needs_context` tested each node for a 3D scene on its own, and the variant in the middle of `Kind` moved the others' discriminants. It's the last variant now, tested with the kinds that need a component context.
