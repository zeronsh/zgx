---
section: fixed
---
- **A box zero on purpose no longer reads as squeezed** (zgx): `layout_warnings`, and so `zgx inspect errors` and the debug banner, said "zero width" of what's inside a box given `w = 0` or `h = 0` (an anchor for what overflows it, a panel slid shut), and of a wrapper sized by such a child. A box squeezed to nothing by its parent is still reported.
