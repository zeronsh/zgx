---
section: performance
---
- **A scroll works out observed bounds from the scroll view up, not from each node** (zgx): every node whose bounds are observed (`NodeHandle::observe_bounds`) keeps its translations summed up to its nearest scroll content, and a frame that only scrolls adds the scroll content's and its ancestors' to that, in the same order, so observers hear the same bounds to the bit. Any other translation of a node with children, or a node made scroll content or not, works them out again; what was kept goes at rest (`Ui::rest`). In a transcript of rows nested 30 boxes deep with their bounds observed, the bounds pass a frame went from 2.0–2.6 ms to 0.3–0.4 ms on an M5.
