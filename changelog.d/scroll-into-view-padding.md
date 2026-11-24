---
section: fixed
---
- **A node scrolled into view keeps its scroll view's `scroll_padding` clear** (zgx, zgx-script): `el:scroll_into_view` (and `NodeHandle::scroll_into_view`) shows a node within each scroll view's `scroll_padding`, as the web's `scroll-padding` keeps it and as focus already did, and `nearest` leaves alone a box spanning the view, as the web's does. Focus, find and `scroll_into_view` now show a node by one rule.
