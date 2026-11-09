---
section: added
---
- **A scroll view can come to rest on whole pages** (zgx, zgx-script): `paging = true` on `ui.scroll_x` or `ui.scroll`, `View::paging` in Rust. On iOS it is `UIScrollView.pagingEnabled`, UIKit's own. On desktop a touch or a trackpad swipe uses UIKit's spring and the speed that turns a page, a lift going at most one page from where it began; a mouse wheel goes a page a notch. Views that don't page scroll as they always have.
