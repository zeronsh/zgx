---
section: changed
---
- **`truncate` forces one line only when `lines` isn't set** (zgx-script): `lines = 2, truncate = true` is now two lines ending in "…", as Rust's `.line_clamp(2).text_overflow(Ellipsis)` is; it was one line. `truncate` alone is still one line, and a wrapper passing both should check the `lines` it sends. A `lines` inherited from a parent counts as set.
