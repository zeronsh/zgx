---
section: performance
---
- **A scroll view resized lays out once** (zgx): its viewport and content took the width a pass had given the view only after that pass, so every resize (a split dragged, a window narrowed) laid out again what it held. A vertical scroll's viewport is now all of the view and its content stretched across it, sized in the layout that sizes the view. A split between six panes dragged costs 1.3% fewer instructions.
