---
section: added
---
- **A span's icon can sit on a well of its own**: `ui.span { icon = { …, bg = "$well", radius = 4, pad = 2 } }` draws a rounded fill under the icon, `pad` past it on every side, over the span's own background, and the line keeps its room. A file chip's icon well, as Zeron's shipping app draws it, with nothing stacked. With no `bg`, nothing changes.
- **A span's padding can differ before and after its text**: `padding = { before, after, down }` (`x` and `{ across, down }` as before). A chip's icon close to its edge, its name with more room after it.
- **`focus_on_press = false`: a press on a node leaves focus where it is** (as the web's `mousedown` with `preventDefault()`): its clicks still come. A list hanging from a field takes clicks on its rows and tabs while the field keeps the keys and its caret.
