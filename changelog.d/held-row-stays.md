---
section: fixed
---
- **A row of a list with given heights that eases to no height stays the same row** (zgx): a virtualized list (`variable_virtual_list`, `ui.list { heights = … }`) built no row thinner than a hundredth of a point, so a row already showing that eased to nothing, as an app hides a row held in a drag, was taken down and built anew as it grew back: a new node, its `use_state`, focus and scroll lost. A row already built now stays while it's thin; rows thin from the start, folded away, are still never built.
