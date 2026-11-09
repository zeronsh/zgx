---
section: performance
---
- **A scroll view draws from a picture of its content while it scrolls** (zgx, zgx-gpu, zgx-desktop): a scroll frame's cost no longer depends on what its rows hold (fades, layers, a blur over the view), and at rest the picture goes. `zgx inspect switch scroll_picture off` stops new ones, to compare.
