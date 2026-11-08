---
section: fixed
---
- **An end-anchored list follows again only when moved to its end** (zgx): as a followed scroll view does, a list with `VariableHeights::anchor_end` (`ui.list { follow = true }`) left above its end stays where it is once rows shrinking bring the end up to it, and a move to the end or a wheel down there follows again. Through a clamp a frame's layout passes on the way to settling (rows measured one pass after another), it keeps its anchor row, so it comes back where it was if the content does.
