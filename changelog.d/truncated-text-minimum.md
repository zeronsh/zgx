---
section: changed
---
- **Text that ends in an ellipsis shrinks past its longest word** (zgx): a text or paragraph whose overflow is an ellipsis (`truncate()`, `text_overflow(Ellipsis)`, `truncate = true` in Luau) has an automatic minimum width of 0 as a flex item or grid item, as one that clips does, so a truncated label in a squeezed row or a `1fr` track gives way and shows its ellipsis instead of pushing its siblings. CSS's `text-overflow: ellipsis` needs `overflow: hidden`, which does the same. It is the text itself: a box around a truncated label still needs the label's width (give the box `min_w(0.)`), and text inheriting `truncate` from a box shrinks as its own would. Set `min_w` on a label that should keep a width.
