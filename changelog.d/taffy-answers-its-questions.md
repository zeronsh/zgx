---
section: fixed
---
- **A child of a box laid out by taffy keeps the size it has** (zgx): taffy's cache took a box's min-content answer for its answer at that width, and a box sized by its content whose text wraps at its min-content width was given the height it has at no room: a line too few, after its container was laid out under other constraints (a probe, then its layout) with the row unchanged.
