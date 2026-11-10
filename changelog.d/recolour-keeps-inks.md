---
section: performance
---
- **A colour changed alone keeps the cached inks of the node and what holds it** (zgx): a rect's, a text's or a quad's colour animating, and rich text whose runs change colour alone (a fade's frame), mark their paint dirty and keep the inks and bounds of the nodes above them, so the frame's damaged walk no longer works them out again up the whole chain. Where a node paints never depends on its colour; any other change clears them as before.
