---
section: added
---
- **Native modules** (zgx-script's `modules` feature, the new `zgx-module` crate): an app's own Rust built apart from zgx as a library and loaded as the app starts (`ScriptHost::load_module`), its functions and actions called from Luau as a host's own are (`ui.call_sync`, `ui.action`). A module links `zgx-module` and nothing of zgx, so a change to a module rebuilds that module alone. It reads the runtime's values where they are (a string's text, a list's items, a map's fields), with no call and no copy: a call into a loaded module costs 10 instructions more than the same function linked in. Only the app's Rust loads one: no script or plugin can. [The design](docs/design/runtime.md) says where this goes.
