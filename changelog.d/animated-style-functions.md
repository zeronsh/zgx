---
section: performance
---
- **A frame of an animation or a tween runs none of the node's style functions** (zgx-script): animating a layout or paint property (`anime.animate(el, { w = … })`), or a `ui.tween` or state beside them, resolved all the node's styles again each frame, so each `bg = function() … end` and the like ran in Luau on every frame though nothing it read had changed. From the first time that would happen, a node's functions resolve apart, when what they read changes. A node not run again pays nothing for this.
