---
section: fixed
---
- **A plugin's node reads what the app provides above it** (zgx-script): `ui.use_context` in what `ui.contribute` adds no longer crashes with "Lua instance passed Value created from a different main Lua state". A plain value arrives converted into the plugin's script, a state as the same state (read only, followed live), and a function is an error naming it. Adding `before` or `after` the root of a `ui.each` or `ui.list` row, which drew nothing, is now an error saying to target an element inside the row.
