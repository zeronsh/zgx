---
section: changed
---
- **A `layout` box no longer eases a size change under a hundredth of a point** (zgx-script): it takes the new size at once. Text measured again or a shrinking parent's share moved it by thousandths of a point, and easing that laid the box out again each frame for nothing.
