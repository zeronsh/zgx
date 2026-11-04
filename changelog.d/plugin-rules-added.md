---
section: added
---
- **`ScriptHost::reserve_prefix(prefix)`** (zgx-script): keeps `prefix` and `prefix.…` for the app's own scripts: no plugin, whatever it is named or granted, offers a `ui.slot` or declares a `ui.command` `id` so named.
- **`ScriptHost::guard(|kind, name| …)`** (zgx-script, with `GuardKind`): asked before the app's own scripts reach a host action, async action or function (by name), or a `ui.fetch`, remote picture or `ui.socket` (by URL), each time; an `Err` stops it, its text the script's error. For previewing someone's template in the app's VM. A host that sets none reads one thread-local flag at each.
