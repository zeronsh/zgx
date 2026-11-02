---
section: added
---
- **A screen reader moving to a node in a scroll view scrolls it in** (zgx-desktop, zgx): nodes in a scroll view offer AccessKit's `ScrollIntoView` (macOS `AXScrollToVisible`, UIA's and AT-SPI's scroll-into-view), and the tree with the node's new place goes out at once, not paced as geometry is. `Ui::scroll_into_view(node)` does the same for an app.
