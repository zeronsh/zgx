---
section: added
---
- **`ui.popover { anchor_fill = true }`** (zgx, zgx-script): the popover's anchor slot takes its share of its row like any child (it grows and stretches with the row), so an anchor with `grow` or `w_full`, a composer's textarea, fills the row with no `on_resize` measuring and its second layout pass. Without it the slot hugs its content as before. `View::anchor_fill` for Rust callers.
