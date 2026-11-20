---
section: added
---
- **`ui.command { menu = "Edit" }` alone declares a menu** (zgx-script, zgx-run): with a menu title and no `id`, `title`, `key` or `run`, it puts the menu in the bar where it is first named and adds no entry, so an app fixes its menu order without a separator. `ScriptHost::commands` lists it with an empty id and no separator flag; the palette and the menus skip it.
