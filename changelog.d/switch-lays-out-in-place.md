---
section: changed
---
- **An unstyled `switch` lays out as what it shows, in its place** (zgx): its view's percentages, `grow` and alignment reach the region's parent, as `.contents()` does and as Luau's `ui.switch` always has, so `h_full` inside it resolves against the parent instead of an auto-height box (a virtual list there built no rows at all). This changes a default: what such a `switch` shows now has its `grow` and percentages reach the parent. Any style on the `switch` (`.w_full()`, `.h(100.)`) keeps a box of its own, as `.not_contents()` does with none.
