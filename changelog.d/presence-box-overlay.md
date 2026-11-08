---
section: performance
---
- **A `ui.presence` with sizing of its own lays out without taffy** (zgx-script): its box is an overlay rather than a one-cell grid, laid out the same way, so presences nested around each paragraph of a chat no longer each run a grid layout. A `layout` box no longer eases a size change under a hundredth of a point (text measured again, a shrinking parent's share), which laid it out again each frame for nothing.
