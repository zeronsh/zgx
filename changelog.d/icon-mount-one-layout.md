---
section: performance
---
- **An icon mounts in the frame's one layout pass** (zgx): an SVG view's artwork fills its box from the start, rather than taking its box's size once laid out and asking for a second pass; a row of icons lays out as a row of plain boxes does. An icon's artwork also stays in its own box under a `relative` ancestor (an image's or a canvas's drawing too).
