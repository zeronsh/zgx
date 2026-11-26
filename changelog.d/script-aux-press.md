---
section: added
---
- `on_aux_press` and `on_aux_release` in scripts: the middle button (or another beyond the primary and secondary) going down on a node and coming back up, told what `on_aux_click` is. The innermost node with one takes the press, and a node with `on_aux_release` or `on_pointer_move` too keeps the pointer until the button comes up, wherever it goes, as `on_press` does for the primary: a board pans while the middle button is held and dragged, as Figma's does. A cancelled gesture releases it with `cancelled = true`.
