---
section: performance
---
- **A list scrolled wholly out of view works out nothing of its clips** (zgx): a list that scrolls with the view around it (`virtualize_in_ancestor`) found what of it shows from its clips every time the view moved, even when it was wholly above or below the view, as most of a long transcript's lists are. Such a list is now told past without them. On a fast scroll through 60 groups of rows (the `scroll-glass` cell) that's about 1% fewer instructions a frame.
