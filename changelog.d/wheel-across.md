---
section: added
---
- **A mouse wheel can move a `scroll_x`** (zgx, zgx-script, zgx-desktop): `View::wheel_across`, Luau `wheel_across = true`. A wheel has no sideways axis, so with Shift it moves the view (and never the page), and without it moves the view once the pointer has been moved onto it, going on to the views around it at the view's edge. A view that slid under a resting pointer leaves the wheel to the page. Windows and Linux report Shift and the wheel as vertical; a Mac reports it as sideways, which every `scroll_x` already took. A trackpad's swipe mostly down leaves such a view still.
- **Markdown tables say how their columns align** (zgx-script): a `table` block has `align`, each column's `"start"`, `"center"`, `"end"`, or `""` where the source says none.
