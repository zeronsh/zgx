---
section: fixed
---
- **A canvas past the 32 MiB raster budget no longer freezes its window** (zgx-gpu, zgx-desktop): it draws coarser, as fine as fits the budget (down to an eighth of the display's scale; past that it alone is left undrawn), and the frame around it draws as usual. Before, its error skipped the whole window's frame, every frame while it stayed that large, and nothing woke the event loop to try again: an app with a long chart (1044 × 6356 points at 2×) stopped presenting until unrelated input arrived. A frame that still can't be drawn is now tried again a second on, the loop woken for it.
