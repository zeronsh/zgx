---
section: added
---
- **Where a plugin can put something and what it can read there** (zgx-script): `ScriptHost::elements` lists the app's ids, classes and hooks grouped by each element that `provide`s context, with a selector that reaches each group and the context names it provides and inherits. A list's repeated rows are one group with a count, private elements show nothing inside, and names the source declares that aren't showing are listed `mounted: false`. `zgx inspect elements` and the MCP tool `zgx_elements` give it as text and JSON.
