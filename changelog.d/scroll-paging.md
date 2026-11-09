---
section: added
---
- **`paging` on `ui.scroll` and `ui.scroll_x`** (`View::paging` in Rust): the view comes to rest only on whole pages, each its own length, as `UIScrollView.pagingEnabled`. On iOS it is UIKit's own paging; elsewhere it scrolls freely for now.
