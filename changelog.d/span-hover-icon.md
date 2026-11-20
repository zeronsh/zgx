---
section: added
---
- **A link span's `hover` swaps its icon and recolours its underline** (zgx, zgx-script): `hover = { icon = { … }, underline_color = … }`, beside `underline`, `color` and `bg`. The `icon` table takes what a span's `icon` does and stands in for it under the pointer (a file pill's download glyph); `underline_color` is the underline's colour there (a darker line). Both are `SpanHover::icon` / `underline_color` for Rust callers. The hover table's type in `zgx.d.luau` now lists `bg` too.
