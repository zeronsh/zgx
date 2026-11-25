---
section: performance
---
- **A scrolled frame's paint list is sized once** (zgx): the walk around what a scroll moves and the window's own shared one size hint, so each frame the big list grew step by step from the small one's size, copying itself, and the small one reserved the big one's room. Each kind of walk keeps its own hint now.
