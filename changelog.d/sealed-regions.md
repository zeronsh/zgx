---
section: added
---
- **Sealed regions** (zgx-script): `ui.sealed { … }` is a region no plugin contributes into, replaces, filters, targets or draws over (a Settings page). In it, `ui.plugin_page { plugin = name }` shows what that plugin alone gives `ui.contribute("plugin_page", …)`, cut to its box, portals included (`zgx::compose::confined`).
