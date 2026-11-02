---
section: added
---
- **A command's shortcut can follow a state** (zgx-script): `ui.command { key = state }` or `key = function() … end` publishes the commands again whenever what it reads changes, so a shortcuts setting rebinds a command at once, menus included, with no reload. While a key follows a state, a key an earlier command already holds stays with it and the later command is published without it, reported in `ScriptHost::dropped_shortcuts` (its `plugin` empty for the app's own), so a rebinding never gives the menu bar a key twice. `ui.shortcut_owner(keys)` names the command holding a key now.
