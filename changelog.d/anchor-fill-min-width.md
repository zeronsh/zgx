---
section: fixed
---
- **A popover's `anchor_fill` slot shrinks as its anchor may** (zgx): the slot filling the anchor's share of its row took no smallest width, so a field that may shrink below its text (`grow`, `min_w = 0`, a chat composer's) stopped at its text's width as a popover's anchor and pushed what followed out of the row. The slot now takes its anchor's `min_w`, as a CSS flex item takes `min-width: 0`.
