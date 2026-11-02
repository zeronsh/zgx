---
section: changed
---
- **A style re-run with the same result no longer resets paint set elsewhere** (zgx): as with an element only changed props touch, an unrelated state change that re-runs a node's reactive style (or a script's style function) to the same result writes nothing, so paint an `anime.animate`, a script or a direct scene write set on the node (its effects, transform, clip) stays until the style's own value for that property changes; then the style's new value wins. Before, such a re-run restored the node's own paint, undoing what was set since (as a style changing only a box's size already did not).
