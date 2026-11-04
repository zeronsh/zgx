---
section: added
---
- **Decision controls no plugin reaches or covers** (zgx-script): `ui.decision { key, … }` holds controls drawn and hit over all a plugin opens, in the room it keeps in flow, scrolling with it.
  - A plugin can't make one, or get into one; replacing, hiding or disabling what declares one is refused and reported.
  - While one is mounted, a plugin's popups, tooltips and context menus stay in the window, under it (`zgx::compose::PopupsInWindow`).
  - `ScriptHost::decision_shown(key)` says whether it is on screen, for a host's fallback in its top layer. In Rust, `zgx::compose::hoist`.
