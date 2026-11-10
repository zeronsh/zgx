---
section: performance
---
- **A sized `ui.svg` or `ui.canvas` is one scene node, not two** (zgx): with a width and height of its own and no padding, border or decoration, its root shows the artwork or drawing itself. A padding, border or decoration arriving later, or a canvas starting to draw text, moves it into a child of the same node once (its id, ref, semantics, hit testing and `on_resize` stay with the node). A canvas with a `view` or a grid keeps its drawing apart, as before. `Scene::leaf_target` names the node that shows it.
