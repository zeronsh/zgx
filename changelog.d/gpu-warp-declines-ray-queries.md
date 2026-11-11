---
section: fixed
---
- **WARP's ray queries are declined** (zgx-gpu, `view-features`): on Microsoft's Basic Render Driver a ray now and then misses the first triangle of an acceleration structure built on WARP's threads, so probes traced there lit a room differently from run to run. A device on WARP no longer asks for ray queries even where they were requested: `ray_queries_status()` says `DeclinedOnAdapter`, and zgx-3d's probes draw faces there. It's decided by WARP's vendor and its device or name alone: a virtualised adapter of Microsoft's with a GPU behind it keeps them.
