---
section: added
---
- **`zgx inspect gc`** (zgx-script): a full Luau collection in each script VM, with the heap before and after (what was garbage) and what is left by object type, userdata type and the chunk each function's code came from (`--top N` rows each). Run it again after the same actions to tell a leak from garbage. `ScriptHost::gc_report`; MCP tool `zgx_gc`.
- **A radial brush for canvases** (zgx, zgx-gpu, zgx-script): `ui.radial(cx, cy, radius, colors)` (Rust `Brush::radial`) fills from its first colour at the centre to its last at the radius and beyond, the colours evenly between, as `ui.linear` does along a line.
