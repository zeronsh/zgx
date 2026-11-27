---
section: fixed
---
- **A driven pointer's cursor is what hover is over** (zgx-desktop): where a driver moves the pointer into the interface alone (`zgx inspect`, an agent, a test) while the real pointer rests elsewhere in the window, the window's cursor now follows the driven point, as hover does, and is found from hover's own hit test. Before, the cursor followed the real pointer, and each frame that moved geometry walked the scene twice, once for each point.
