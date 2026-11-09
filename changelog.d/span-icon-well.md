---
section: added
---
- **A span's icon can sit on a well of its own**: `ui.span { icon = { …, bg = "$well", radius = 4, pad = 2 } }` draws a rounded fill under the icon, `pad` past it on every side, over the span's own background, and the line keeps its room. A file chip's icon well, as Zeron's shipping app draws it, with nothing stacked. With no `bg`, nothing changes.
