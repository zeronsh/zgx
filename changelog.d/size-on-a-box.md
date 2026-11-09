---
section: fixed
---
- **`size` sizes a box, as the reference has always said** (zgx-script): on `ui.div`, `ui.column`, `ui.row`, `ui.grid`, `ui.overlay`, `ui.scroll` and `ui.presence`, `size = 50` or `size = { 120, 60 }` is `w` and `h` at once. It sized nothing on any element: the name is also `ui.split`'s state, and a constructor's fields are read before style properties. A box given a `size` it ignored until now takes that size. Elsewhere it still does nothing, and `zgx lint` says so (`size-inert`), pointing a text to `text_size`.
