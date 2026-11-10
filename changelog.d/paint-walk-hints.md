---
section: performance
---
- **A damaged paint walk no longer reserves room for the whole window** (zgx): each walk sized its list from the window's last walk, so a frame that repainted a strip or two after a whole repaint (a streamed reply's fade, after a chunk scrolled the view) allocated for every item and gave it back. Whole and damaged walks keep their own size hints.
