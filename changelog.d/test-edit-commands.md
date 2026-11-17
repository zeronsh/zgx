---
section: fixed
---
- **The Edit menu's Select All selects a selectable region's text** (zgx-desktop): with no field focused it takes the text last pressed in, as ⌘A does; before, it did nothing there. A copy from a field by a key that reaches the view directly (tests, `zgx inspect key`) now goes through its `on_copy`, as the window's does.
- **A test runs the Edit menu's items** (zgx-script): `app:command("copy")`, `"cut"`, `"paste"` and `"select-all"` do what the desktop's Edit menu does, in the focused field or the selectable text last pressed in, a copy through its `on_copy`, and say whether anything took it.
