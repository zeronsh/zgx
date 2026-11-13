---
section: fixed
---
- **A reactive `outline_color` keeps the node's outline width** (zgx): a function-valued `outline_color` is a style patch of its own, which had no width and so forced one of 1 point over the node's own; `outline = 0` with a function colour drew a 1 px ring. The patch now leaves the width to the node's own (0 included), and a node with no width set still draws 1 point.
