---
section: added
---
- **A GPU view's painter is told when its window goes idle** (zgx-gpu): `Viewport::rest` runs from the renderer's idle trim, once frames have stopped for a moment, so a painter can let go of what only drawing needs (a 3D world's temporary targets) and make it again with its next frame. It defaults to doing nothing. A view whose window keeps drawing other things isn't asked.
