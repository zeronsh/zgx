---
section: fixed
---
- **Text in a `snap` box doesn't wrap and unwrap as the box narrows** (zgx): a box's edges round to whole points separately, so its width came and went by a point as it only narrowed (429, 428, 429, 427), and what it holds was laid out at that: text at its wrap point went down a line and back, several times in a drag. While its size is changing, what it holds is laid out at that size rounded, which goes one way. Its edges, and the gaps between boxes that share them, are where they were, and once it has held still for 300 ms it's laid out as before.
