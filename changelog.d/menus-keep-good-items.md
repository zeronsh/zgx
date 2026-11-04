---
section: fixed
---
- **One bad menu item no longer costs the whole menu bar, and every shortcut with it** (zgx-desktop, zgx-script). In Zeron, a plugin's sheet called `set_menus` without the app's commands, and every shortcut died until relaunch. Three things are fixed:
  - `WindowFactory::set_menus` (and `Application::menus`) refused the whole bar when one item had no label, repeated another's action, or had an accelerator another item held or no key names (⌘é, ⌘?). Now that item is left out, or keeps no shortcut, and the rest is shown. Each problem is written on stderr (`zgx: menus: …`) when it first appears. `set_menus` errs only when the application is closed. Its docs now say it replaces every window's menus at once.
  - A plugin whose script failed as it loaded kept the `ui.command`s and `ui.window`s it had declared before failing. They stayed in the menu bar, and `run_command` returned false for them. Everything it declared now goes with it.
  - On macOS, a menu bar AppKit could not make (`Event::MenusChanged`, or the first bar) ended the app. Now it is reported, the last good bar stays, and a first bar falls back to the standard application and Window menus.
