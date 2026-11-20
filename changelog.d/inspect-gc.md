---
section: added
---
- **`zgx inspect gc`** (zgx-script): a full Luau collection in each script VM, with the heap before and after (what was garbage) and what is left by object type, userdata type and the chunk each function's code came from (`--top N` rows each). Run it again after the same actions to tell a leak from garbage. MCP tool `zgx_gc`.
