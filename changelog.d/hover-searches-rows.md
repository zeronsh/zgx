---
section: performance
---
- **Hover over a long scrolled list finds the row under the pointer by search** (zgx): a plain hit test over a node of 64 children or more, whose boxes run down in order with none transformed, now looks only at the children reaching the pointer's height. The others are skipped because each proves nothing of it reaches past its top or bottom, checked each frame. A frame that scrolls rows under a still pointer runs in 95K instructions at 2,000 rows instead of 278K, and in 341K at 8,000 instead of 1,057K, and finds the same nodes.
