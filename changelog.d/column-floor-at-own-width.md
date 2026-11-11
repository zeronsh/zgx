---
section: fixed
---
- **A column shrinks a box no further than its content needs at the box's own width** (zgx): a box narrower than its column (a set width, a share, or centred) had its floor measured with its text at the column's width, a line where it wraps to several, and the column shrank it below what its text needs. Its floor is now its content's height at its own width, less its padding and border, as CSS's `min-height: auto` is.
