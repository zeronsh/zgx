---
section: fixed
---
- **lavapipe's ray queries are declined** (zgx-gpu, `view-features`): Mesa's software Vulkan driver offers ray queries but crashes building the first acceleration structure, so probes traced there took the process down. A device on lavapipe (llvmpipe under Vulkan) no longer asks for them even where they were requested: `ray_queries_status()` says `DeclinedOnAdapter`, and zgx-3d's probes draw faces there.
