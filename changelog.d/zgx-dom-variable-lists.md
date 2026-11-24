---
section: added
---
- **Variable and measured virtual lists in the DOM** (`zgx-dom`): `variable_virtual_list`, `measured_virtual_list` and `measured_rows` keep only the rows in view, each at its own height; measured rows report the height they lay out at (ResizeObserver). As heights above the view change, the first row in view stays where it shows, or the end for a list anchored there.
