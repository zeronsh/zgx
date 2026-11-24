---
section: performance
---
- **A fade drawn as one costs less a frame** (zgx-gpu): each frame that draws a fade's bands as one looked through the whole run of items under it for ids to key its textures (a lookup per item, hundreds in a transcript), and worked out each item's reach three times. It now looks only as far as the textures need, works each reach out once, and clones only the items a by-item strip cuts. What is drawn is the same.
