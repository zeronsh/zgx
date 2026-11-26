---
section: added
---
- **A plugin's modules can be named as the app's** (zgx-script): `Plugin::app_root` names the modules the app runs from that folder of the plugin (a `ui.override` replacement and what it requires) by their path in it, as the app's own are, and their states with them (`ds/map.luau::open`), where they were named by their full path on the disk. A module an app moves into a plugin it ships keeps its saved state, and errors name no folder of the machine's. `Plugin` has the new field: `app_root: None` keeps the full paths.
