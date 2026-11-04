---
section: performance
---
- **Laying out a box whose size is known doesn't measure it as one whose size isn't** (zgx): an absolute box with both insets, and any child flex had already sized, was measured a second time with no width before being placed at it, with everything it holds.
