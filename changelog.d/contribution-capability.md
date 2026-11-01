---
section: added
---
- **A host can ask a capability of every plugin contribution** (zgx-script): `ScriptHost::require_contribution_capability(capability)` makes adding into a slot (`ui.contribute(name, node)`) or before or after an element (by `#id` or a hook) take that capability, so an app can lock plugins out of its interface. A target or slot the host opens with `expose_hook(target, None)` (`"slot:name"` for a slot) stays open. Without it, contributions are open, as before.
