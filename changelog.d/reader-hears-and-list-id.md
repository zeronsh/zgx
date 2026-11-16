---
section: fixed
---
- **A `ui.list` that is itself a selectable region names its rows by its own id** (zgx-script): `on_copy`'s `request.rows` gave `list = false` for `ui.list { id = …, selectable = true, … }`; the list's own id counts now, as it does for a list inside a region. `zgx inspect node` on a paragraph with a citation (or any labelled control) in it also says what a screen reader hears, `reads: "step " [button "Citation 1"] "."`, from the same code the desktop's accessibility tree is built with.
