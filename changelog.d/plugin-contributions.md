---
section: added
---
- **What each plugin contribution does now** (zgx-script): `ScriptHost::contributions()` lists each `ui.contribute { target }` and `ui.filter` with its plugin, target, place, how many elements it applies at now (0 when its target matches nothing shown) and its latest error. `zgx inspect contributions` and the MCP tool `zgx_contributions` give the same, as text and JSON.
