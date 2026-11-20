---
section: added
---
- **`zgx inspect gc` counts nodes alive and what the script keeps for its life** (zgx-script): "nodes alive" (script nodes made and not dropped, against the node userdata the VM still holds: more of the first after full collections means a Rust holder keeps them) and "kept for the script's life" (resources made at no scope, and ended timers among them). A script host also asks its window for the idle trim (a full collection) every 4,096 nodes made, so a window that never goes idle doesn't leave the collector's lag (node userdata pins closures and their upvalues until two cycles pass) to pile up.
