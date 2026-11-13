---
section: performance
---
- **A removed subtree is named to a renderer without copying it** (zgx): removing a node copied each node's children under it into a list of its own, to name them to a renderer that drops what they held (`Scene::resource_changes`). The subtree is now walked into the list it is named in, which keeps its room between flushes. A fast scroll through deep rows (scroll-glass) allocates about 30 times a frame fewer.
