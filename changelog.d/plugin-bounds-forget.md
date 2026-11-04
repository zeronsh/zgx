---
section: added
---
- **`ScriptHost::contribution_bounds(plugin)`** (zgx-script, with zgx's `NodeHandle::window_bounds`): where what a plugin added shows in the window now, a box for each thing mounted in a slot or before, after or in place of an element, through any transforms and zooms. What a `ui.filter` changes stays the app's own. Costs nothing without plugins.
- **`ScriptHost::forget_state(script)`** (zgx-script): for uninstalling a plugin, `"plugin:<name>"` forgets its `ui.state` and `ui.persist` values, what waits to be saved and its saved files, gone by the time it returns; a write already queued can't bring them back.
