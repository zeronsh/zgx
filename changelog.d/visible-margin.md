---
section: added
---
- **`visible_margin` on `on_visible`** (zgx-script, `NodeHandle::observe_visibility_within` in zgx): an element counts as in view once it comes within that many points of showing, as an IntersectionObserver's `rootMargin` does. It's judged natively as the view scrolls, and calls `on_visible` only as an element crosses, so a script can build what is about to scroll in without reading the scroll position every frame.
