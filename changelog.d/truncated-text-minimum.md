---
section: changed
---
- **Text that ends in an ellipsis shrinks past its longest word** (zgx): `truncate()` (and `text_overflow(Ellipsis)`, `truncate = true` in Luau) gives a flex item an automatic minimum width of 0, as clipping does, so a truncated label in a squeezed row gives way and shows its ellipsis instead of pushing its siblings or overflowing. CSS's `text-overflow: ellipsis` needs `overflow: hidden`, which does the same. Set `min_w` on a label that should keep a width.
