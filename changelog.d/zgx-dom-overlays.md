---
section: added
---
- **Overlays in the DOM** (`zgx-dom`): `modal` is a `<dialog>` shown with `showModal()`; `popover`, `menu`, `submenu` and tooltips are popovers in the browser's top layer, placed against their anchors. Escape, a click outside and focus behave as the browser's own, and their closing sets the view's `open` signal. Menus move among their items with the arrows and open submenus with Right or the pointer; choosing an item closes its menus before its click runs. The same view code as on the desktop.
