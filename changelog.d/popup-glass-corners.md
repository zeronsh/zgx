---
section: fixed
---
- **A rounded popup's glass stays inside its curve on Windows** (zgx-gpu): past the corners of a popup with `backdrop_blur`, the popup drew a wedge of its window's frame, opaque, over its own shadow, because the copy of what lies under the blur came from the window and not the popup. Frames drawn at presentation, as Windows draws them, took that path; those mixed by blending never did.
